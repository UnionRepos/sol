use sol_css::syntax::{self, ComponentValue as C, ErrorKind, TokenKind as T};

fn tokens(input: &str) -> Vec<T> {
    syntax::tokenize(input)
        .tokens
        .into_iter()
        .map(|t| t.kind)
        .filter(|t| *t != T::Whitespace)
        .collect()
}

#[test]
fn escapes_unicode_and_preprocessing() {
    assert_eq!(
        tokens(r"caf\e9  \31 23 --A \0  \d800  \110000  x\"),
        vec![
            T::Ident("café".into()),
            T::Ident("123".into()),
            T::Ident("--A".into()),
            T::Ident("�".into()),
            T::Ident("�".into()),
            T::Ident("�".into()),
            T::Ident("x�".into()),
        ]
    );
    let parsed = syntax::tokenize("a\r\nb\rc\u{c}d\0é");
    assert_eq!(parsed.source, "a\nb\nc\nd�é");
    for token in parsed.tokens {
        assert!(parsed.source.is_char_boundary(token.start));
        assert!(parsed.source.is_char_boundary(token.end));
    }
    assert_eq!(tokens("a\u{a0}b"), vec![T::Ident("a\u{a0}b".into())]);
}

#[test]
fn number_lookahead_and_unknown_units() {
    let got = tokens("+.5e+2PX -2.0 1e-3% 10furlong 1e + - . #123 #abc");
    assert!(matches!(&got[0], T::Dimension(n,u) if n.value == 50.0 && !n.integer && u == "PX"));
    assert!(matches!(&got[1], T::Number(n) if n.value == -2.0 && !n.integer));
    assert!(matches!(&got[2], T::Percentage(n) if n.value == 0.001));
    assert!(matches!(&got[3], T::Dimension(n,u) if n.value == 10.0 && u == "furlong"));
    assert!(matches!(&got[4], T::Dimension(_,u) if u == "e"));
    assert!(matches!(&got[8], T::Hash { id: false, .. }));
    assert!(matches!(&got[9], T::Hash { id: true, .. }));
}

#[test]
fn strings_urls_and_comments_have_distinct_recovery() {
    assert_eq!(
        tokens("\"a;}:,b\" 'x\\\ny' url(a\\ b) url(\"a b\")"),
        vec![
            T::String("a;}:,b".into()),
            T::String("xy".into()),
            T::Url("a b".into()),
            T::Function("url".into()),
            T::String("a b".into()),
            T::Close(')'),
        ]
    );
    let parsed = syntax::tokenize("url(a b) x \"bad\n good; /*unterminated");
    assert!(parsed.errors.iter().any(|e| e.kind == ErrorKind::BadUrl));
    assert!(parsed.errors.iter().any(|e| e.kind == ErrorKind::BadString));
    assert!(parsed
        .errors
        .iter()
        .any(|e| e.kind == ErrorKind::UnterminatedComment));
    assert!(parsed
        .tokens
        .iter()
        .any(|t| t.kind == T::Ident("good".into())));
    assert_eq!(
        tokens("red/**/blue"),
        vec![T::Ident("red".into()), T::Ident("blue".into())]
    );
}

#[test]
fn unicode_ranges_are_contextual() {
    let parsed = syntax::tokenize_with_unicode_ranges("U+4?? U+00a0-00ff u+1234567", true);
    let got: Vec<_> = parsed
        .tokens
        .into_iter()
        .map(|t| t.kind)
        .filter(|t| *t != T::Whitespace)
        .collect();
    assert!(matches!(got[0], T::UnicodeRange(0x400, 0x4ff)));
    assert!(matches!(got[1], T::UnicodeRange(0xa0, 0xff)));
    assert!(matches!(got[2], T::UnicodeRange(0x123456, 0x123456)));
    assert!(matches!(got[3], T::Number(_)));
    assert!(!tokens("U+4??")
        .iter()
        .any(|t| matches!(t, T::UnicodeRange(..))));
}

fn shape(values: &[C]) -> Vec<String> {
    values
        .iter()
        .map(|v| match v {
            C::Token(t) => format!("{:?}", t.kind),
            C::Function { name, value } => format!("{name}({:?})", shape(value)),
            C::Block { opening, value } => format!("{opening}{:?}", shape(value)),
        })
        .collect()
}

#[test]
fn serialization_preserves_token_boundaries() {
    for input in [
        r"1\65 2 2\45-3 3px red/**/blue .a/**/.b #\31 23 #1\a b",
        "f(a/**/(b)) url(a\\ b) \"a\\a b\"",
        "--x: a/**/-/**/-; [x|=en]",
    ] {
        let original = syntax::parse_component_values(input);
        let serialized = syntax::serialize(&original.values);
        let reparsed = syntax::parse_component_values(&serialized);
        assert_eq!(
            shape(&original.values),
            shape(&reparsed.values),
            "{input} -> {serialized}"
        );
    }
}

#[test]
fn arbitrary_at_rules_and_nested_blocks_survive() {
    let input = "<!-- @charset \"utf-8\"; @layer a, b; @container (width > 5px) { .x { & > a { --theme: {a:b}; } } } @keyframes fade { 0% { opacity:0 } to { opacity:1 } } @future x(y[z]) { unknown: f(1;2); } -->";
    let parsed = syntax::parse_stylesheet(input);
    assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    assert_eq!(parsed.rules.len(), 5);
    for rule in parsed.rules {
        let serialized = rule.to_css();
        let reparsed = syntax::parse_rule(&serialized).unwrap();
        assert_eq!(reparsed.to_css(), serialized);
    }
    assert_eq!(syntax::parse_stylesheet("{}").rules.len(), 1);
    assert!(syntax::parse_rule("a{} b{}").is_err());
    assert!(syntax::parse_component_value("a b").is_err());
    assert!(syntax::parse_component_value("f(1").is_ok());
}

#[test]
fn declarations_keep_quoted_delimiters_and_custom_case() {
    let parsed=syntax::parse_block_contents("color: red ! /*x*/ IMPORTANT; content: 'x;}:!important'; --Theme: { x: [1;2] }; --empty:; nonsense; width: 5px; & > a {color:blue} @media (width>0px){a{}} height:6px");
    assert_eq!(parsed.items.len(), 8);
    assert_eq!(parsed.errors.len(), 1);
    let syntax::BlockItem::Declaration(first) = &parsed.items[0] else {
        panic!()
    };
    assert!(first.important);
    let syntax::BlockItem::Declaration(custom) = &parsed.items[2] else {
        panic!()
    };
    assert_eq!(custom.name, "--Theme");
    assert!(syntax::parse_declaration("color:")
        .unwrap()
        .value
        .is_empty());
    for invalid in [
        "color red",
        "x:red; y:blue",
        "x:red !oops",
        "--:x",
        "x:url(a b)",
        "x:]",
    ] {
        assert!(syntax::parse_declaration(invalid).is_err(), "{invalid}");
    }
    assert!(syntax::parse_declaration("content:'!important'").is_ok_and(|d| !d.important));
}

#[test]
fn eof_recovery_and_depth_limits_are_reported() {
    let parsed = syntax::parse_stylesheet("a { color:red; transform:f([1");
    assert_eq!(parsed.rules.len(), 1);
    assert_eq!(
        parsed
            .errors
            .iter()
            .filter(|e| e.kind == ErrorKind::UnclosedBlock)
            .count(),
        3
    );
    let deep = format!("{}x{}", "f(".repeat(1000), ")".repeat(1000));
    assert!(syntax::parse_component_values(&deep)
        .errors
        .iter()
        .any(|e| e.kind == ErrorKind::LimitExceeded));
}

#[test]
fn deterministic_malformed_unicode_corpus_never_panics() {
    let alphabet = [
        'é', '😀', '\0', '\r', '\n', '\\', '"', '\'', '{', '}', '[', ']', '(', ')', ';', ':', '@',
        '#', '/', '*', '+', '-', '!', '0', '9', 'a', 'n', ' ',
    ];
    let mut seed = 0x1234_5678u64;
    for _ in 0..1500 {
        let mut input = String::new();
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            input.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
        }
        syntax::parse_stylesheet(&input);
        syntax::parse_block_contents(&input);
        sol_css::parse_stylesheet(&input);
        sol_css::parse_selector_list(&input);
        sol_css::parse_value(&input);
    }
}

#[test]
fn standalone_curly_values_and_unicode_range_descriptors() {
    let declaration = syntax::parse_declaration("future: {a:b} !important").unwrap();
    assert!(matches!(
        declaration.value.as_slice(),
        [C::Block { opening: '{', .. }]
    ));
    assert!(syntax::parse_declaration("future:x {a:b}").is_err());
    assert!(matches!(
        &syntax::parse_block_contents("future:{a:b}; color:red").items[0],
        syntax::BlockItem::Declaration(_)
    ));
    let declaration = syntax::parse_declaration("unicode-range:U+4??, U+00a0-00ff").unwrap();
    assert_eq!(
        declaration.value[0].kind(),
        Some(&T::UnicodeRange(0x400, 0x4ff))
    );
    let separated = syntax::parse_declaration("unicode-range:U/**/+4??").unwrap();
    assert!(!separated
        .value
        .iter()
        .any(|v| matches!(v.kind(), Some(T::UnicodeRange(..)))));
    let custom = syntax::parse_declaration("--range:U+4??").unwrap();
    assert!(!custom
        .value
        .iter()
        .any(|v| matches!(v.kind(), Some(T::UnicodeRange(..)))));
}
