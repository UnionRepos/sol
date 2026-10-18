use sol_css::{
    parse_selector, parse_selector_list, parse_stylesheet, parse_value,
    style::{self, viewport},
    NoState, Style, Value,
};
use sol_dom::{Document, NodeId};

fn document(html: &str) -> Document {
    let mut doc = Document::new();
    sol_html::parse(html, &mut doc);
    doc
}
fn find(doc: &Document, id: &str) -> NodeId {
    doc.get_element_by_id(id).unwrap()
}
fn matches(doc: &Document, id: &str, selector: &str) -> bool {
    style::matches(
        doc,
        find(doc, id),
        &parse_selector(selector).unwrap(),
        &NoState,
    )
}
fn styled(css: &str) -> Style {
    let doc = document("<div id=p><span id=x></span></div>");
    let sheet = parse_stylesheet(css);
    sol_css::style_document(&doc, &[&sheet], &NoState)[&find(&doc, "x")]
        .as_ref()
        .clone()
}

#[test]
fn strict_lists_and_forgiving_pseudos() {
    for invalid in [
        "",
        ".",
        "#123",
        "div >",
        "div >> p",
        "div[",
        "[x=]",
        "[x=\"a\" q]",
        "div?",
        ":made-up",
        ":not(.ok, ?)",
        ":nth-child(2n of)",
        ":nth-of-type(2n of .a)",
        ":nth-child(2.0)",
        ":nth-child(2 n)",
        ":nth-child(2/**/n)",
        ":nth-child(+/**/2n)",
        ":nth-child(+ n)",
        ":nth-child(2147483648)",
        ":has(:has(a))",
        ":has(::before)",
        ":has(:is(:has(a)))",
        ":not(::before)",
        "p::before.x",
        "p::before > b",
    ] {
        assert!(parse_selector(invalid).is_none(), "{invalid}");
    }
    assert!(parse_selector_list(".ok, .").is_empty());
    assert!(parse_selector(":is(.ok, :made-up, ?)").is_some());
    assert!(parse_selector(":where(?)").is_some());
    assert_eq!(parse_selector_list(".a,.b").len(), 2);
}

#[test]
fn selectors_preserve_escapes_and_attribute_flags() {
    let doc = document(r#"<p id="1:a" class="café a:b" data-text="A B;]," lang="en-US"></p>"#);
    for selector in [
        r"#\31 \:a",
        r".caf\e9 ",
        r".a\:b",
        r#"[data-text="a b;]," i]"#,
        r#"[data-text="a b;],"i]"#,
        "[lang|=en]",
        "[lang^=en]",
        "[lang$=US]",
        "[lang*=n-U]",
        "[class~=café]",
    ] {
        assert!(matches(&doc, "1:a", selector), "{selector}");
    }
    for selector in [
        r#"[data-text="a b;]," s]"#,
        "[lang^='']",
        "[lang$='']",
        "[lang*='']",
        "[class~='']",
        "#other",
        "#other#\\31 \\:a",
    ] {
        assert!(!matches(&doc, "1:a", selector), "{selector}");
    }
}

#[test]
fn level_four_specificity() {
    for (selector, expected) in [
        ("div:where(#id,.class)", (0, 0, 1)),
        (":is(.a,#b)", (1, 0, 0)),
        ("p:not(.a,#b)", (1, 0, 1)),
        ("section:has(> #a, + .b)", (1, 0, 1)),
        ("li:nth-child(2n of .x,#id)", (1, 1, 1)),
        ("#x#x.a::before", (2, 1, 1)),
    ] {
        assert_eq!(
            parse_selector(selector).unwrap().specificity,
            expected,
            "{selector}"
        );
    }
    assert_eq!(
        styled("#x{color:red} :where(#x){color:blue}").color("color"),
        Some([255, 0, 0, 255])
    );
}

#[test]
fn relational_selectors_are_anchored_to_subject() {
    let doc=document("<main><section id=a><p><b class=deep></b></p></section><section id=b><i></i></section><section id=c></section></main>");
    for selector in [
        "section:has(> p)",
        "section:has(p > .deep)",
        "section:has(+ section > i)",
        "section:has(~ #c)",
        "section:is(#a,#z):not(#b,#c)",
    ] {
        assert!(matches(&doc, "a", selector), "{selector}");
    }
    for selector in [
        "section:has(> .deep)",
        "section:has(+ #c)",
        "section:has(#a)",
        "section:has(section .deep)",
        "section:has(+ section > p)",
    ] {
        assert!(!matches(&doc, "a", selector), "{selector}");
    }
    let selector = parse_selector(":scope > p").unwrap();
    let p = doc.children(find(&doc, "a"))[0];
    assert!(style::matches_scoped(
        &doc,
        p,
        &selector,
        &NoState,
        Some(find(&doc, "a"))
    ));
}

#[test]
fn filtered_nth_and_reverse_type_positions() {
    let doc=document("<ul><li id=a class=x></li><i></i><li id=b></li><li id=c class=x></li><li id=d class=x></li></ul>");
    for selector in [
        "li:nth-child(2 of .x)",
        "li:nth-last-child(2 of .x)",
        "li:nth-last-of-type(2)",
        "li:nth-of-type(-n + 3)",
        "li:nth-child(2n)",
        "li:nth-child(n-1)",
    ] {
        assert!(matches(&doc, "c", selector), "{selector}");
    }
    assert!(!matches(&doc, "b", "li:nth-child(2 of .x)"));
    assert!(matches(&doc, "a", "li:nth-child(-2147483648n + 1)"));
}

#[test]
fn empty_and_inherited_language() {
    let doc = document("<main lang=en-US><p id=a> </p><p id=b><!--x--></p></main>");
    assert!(!matches(&doc, "a", ":empty"));
    assert!(matches(&doc, "b", ":empty:lang(fr,en)"));
}

#[test]
fn declarations_recover_and_important_is_token_based() {
    let decls=sol_css::parse_decls("content:'a;}!important'; bad; color:blue ! /**/ ImPoRtAnT; width:url(a b); height:10px; --Case:foo; --case:bar");
    assert_eq!(decls.len(), 5);
    assert_eq!(decls[0].value, vec![Value::Str("a;}!important".into())]);
    assert!(!decls[0].important);
    assert!(decls[1].important);
    assert_eq!(decls[3].name, "--Case");
}

#[test]
fn variables_resolve_after_cascade_and_before_shorthands() {
    let style=styled("#p{--Space:3px 7px; --RGB:255 0 0; --size:12px} #x{margin:var(--Space); width:calc(var(--size) * 2 + 1px); color:rgb(var(--RGB) / 50%); --size:20px; --RGB:0 0 255}");
    assert_eq!(style.length_px("margin-top", 16.0, 800.0), Some(3.0));
    assert_eq!(style.length_px("margin-right", 16.0, 800.0), Some(7.0));
    assert_eq!(style.length_px("width", 16.0, 800.0), Some(41.0));
    assert_eq!(style.color("color"), Some([0, 0, 255, 128]));
}

#[test]
fn variables_are_case_sensitive_and_inherit_computed_values() {
    let style = styled(
        "#p{--A:10px; --B:var(--A); --A:12px} #x{--A:40px; width:var(--B); height:var(--a,8px)}",
    );
    assert_eq!(style.length_px("width", 16.0, 800.0), Some(12.0));
    assert_eq!(style.length_px("height", 16.0, 800.0), Some(8.0));
}

#[test]
fn variable_cycles_fallbacks_and_invalid_computed_values() {
    let style=styled("#p{color:red} #x{--a:var(--b); --b:var(--a,4px); width:var(--a,var(--missing,7px)); height:5px; height:var(--none); color:blue; color:var(--none); --empty:; padding:var(--missing,); --self:var(--ok,var(--self)); top:var(--self,9px)}");
    assert_eq!(style.length_px("width", 16.0, 800.0), Some(7.0));
    assert_eq!(style.length_px("height", 16.0, 800.0), None);
    assert_eq!(style.length_px("top", 16.0, 800.0), Some(9.0));
    assert_eq!(style.color("color"), Some([255, 0, 0, 255]));
    assert!(style.get("--a").is_none());
    assert!(style.get("--empty").is_some());
}

#[test]
fn variable_substitution_never_merges_adjacent_tokens() {
    let style = styled("#x{--n:10; width:var(--n)px; --text: 'MiXeD'; content:var(--text)}");
    assert_eq!(style.length_px("width", 16.0, 800.0), None);
    assert_eq!(
        style.get("content").unwrap(),
        &vec![Value::Str("MiXeD".into())]
    );
}

#[test]
fn math_functions_have_precedence_units_and_validation() {
    for (value, expected) in [
        ("calc(10px + 2 * 5px)", 20.0),
        ("calc((100% - 20px) / 2)", 390.0),
        ("min(30px,2em)", 30.0),
        ("max(10px,3em)", 48.0),
        ("clamp(20px,50%,300px)", 300.0),
        ("clamp(30px,5px,20px)", 30.0),
        ("calc(-1px + max(2px,4px))", 3.0),
    ] {
        assert_eq!(
            style::value_px(&parse_value(value)[0], 16.0, 800.0),
            Some(expected),
            "{value}"
        );
    }
    for value in [
        "calc(1px+2px)",
        "calc(1px +2px)",
        "calc(1px * 1px)",
        "calc(1px + 1)",
        "calc(10px / 0)",
        "calc(10px / 1px)",
        "calc(1s + 1s)",
        "min()",
        "clamp(1px,2px)",
        "calc(2)",
        "1furlong",
        "2s",
    ] {
        assert_eq!(
            style::value_px(&parse_value(value)[0], 16.0, 800.0),
            None,
            "{value}"
        );
    }
}

#[test]
fn viewport_units_do_not_use_percentage_base() {
    viewport::WIDTH.set(1000.0);
    viewport::HEIGHT.set(500.0);
    assert_eq!(
        style::value_px(&parse_value("10vw")[0], 16.0, 200.0),
        Some(100.0)
    );
    assert_eq!(
        style::value_px(&parse_value("10vh")[0], 16.0, 200.0),
        Some(50.0)
    );
    viewport::WIDTH.set(800.0);
    viewport::HEIGHT.set(600.0);
}

#[test]
fn modern_colors_and_invalid_color_functions() {
    for (value, color) in [
        ("rgb(100% 0% 0% / 50%)", [255, 0, 0, 128]),
        ("hsl(-120deg 100% 50%)", [0, 0, 255, 255]),
        ("hsl(.5turn 100% 50%)", [0, 255, 255, 255]),
        ("hwb(0 0% 0% / 25%)", [255, 0, 0, 64]),
        ("#abcd", [170, 187, 204, 221]),
    ] {
        assert_eq!(
            parse_value(value),
            vec![Value::Color(color[0], color[1], color[2], color[3])],
            "{value}"
        );
    }
    for value in [
        "rgb(1,,2,3)",
        "rgb(1 2 3 0.5)",
        "rgb(1,2,3 / .5)",
        "rgb(10%,2,3)",
        "hwb(0,0%,0%)",
    ] {
        assert!(matches!(&parse_value(value)[0], Value::Func(..)), "{value}");
    }
}

fn media(input: &str) -> bool {
    let parsed = sol_css::syntax::parse_component_values(input);
    sol_css::conditions::media_matches(&sol_css::conditions::parse_media(&parsed.values))
}
fn supports(input: &str) -> bool {
    sol_css::conditions::parse(&sol_css::syntax::parse_component_values(input).values).supports()
}

#[test]
fn media_boolean_ranges_ratios_and_query_negation() {
    viewport::WIDTH.set(800.0);
    viewport::HEIGHT.set(600.0);
    for query in [
        "screen and (width:800px)",
        "(400px < width <= 800px)",
        "(width >= 50em)",
        "(aspect-ratio:4/3)",
        "(resolution:1dppx)",
        "not print and (width:100px)",
        "(width:1px), (height:600px)",
        "((width:1px) or (height:600px))",
        "not (width:1px)",
    ] {
        assert!(media(query), "{query}");
    }
    for query in [
        "(width:1px)",
        "not screen and (width:800px)",
        "(width:10s)",
        "not only",
        "only (width:800px)",
        "screen and (width:800px) or (height:999px)",
        "(width:800)",
        "(400px < width > 500px)",
        "(width < width)",
        "not screen and nonsense",
        "not ((width:800px) and)",
        "(width:800px) or (height:600px) and (color)",
    ] {
        assert!(!media(query), "{query}");
    }
}

#[test]
fn supports_boolean_declarations_selectors_and_unknowns() {
    for condition in [
        "(display:flex)",
        "not (display:grid)",
        "(display:flex) and (color:red)",
        "selector(:where(.a,#b))",
        "(--Theme:)",
        "(width:var(--w))",
    ] {
        assert!(supports(condition), "{condition}");
    }
    for condition in [
        "(display:madeup)",
        "(width:1s)",
        "(width:1px 2px)",
        "(padding:auto)",
        "(madeup:1)",
        "(color:red !important)",
        "selector(:unknown)",
        "not nonsense",
        "not ((display:flex) and)",
        "(display:flex) or (color:red) and (width:1px)",
    ] {
        assert!(!supports(condition), "{condition}");
    }
    assert_eq!(
        styled("@supports(display:flex){#x{color:red}} @supports(display:grid){#x{color:blue}}")
            .color("color"),
        Some([255, 0, 0, 255])
    );
}

#[test]
fn box_shorthands_repeat_opposite_sides_and_use_canonical_longhands() {
    let style=styled("#x{margin:1px 2px; padding:3px 4px 5px; border-width:1px 2px 3px 4px; border-left:6px solid red}");
    for (name, expected) in [
        ("margin-top", 1.0),
        ("margin-right", 2.0),
        ("margin-bottom", 1.0),
        ("margin-left", 2.0),
        ("padding-left", 4.0),
        ("padding-bottom", 5.0),
        ("border-top-width", 1.0),
        ("border-right-width", 2.0),
        ("border-bottom-width", 3.0),
        ("border-left-width", 6.0),
    ] {
        assert_eq!(style.length_px(name, 16.0, 800.0), Some(expected), "{name}");
    }
    assert_eq!(style.color("border-left-color"), Some([255, 0, 0, 255]));
}

#[test]
fn variable_expansion_budget_invalidates_exponential_values() {
    let mut css = String::from("#x{--v0:1px;");
    for i in 1..19 {
        css.push_str(&format!("--v{i}:var(--v{}) var(--v{});", i - 1, i - 1));
    }
    css.push_str("width:var(--v18,7px)}");
    assert_eq!(styled(&css).length_px("width", 16.0, 800.0), Some(7.0));
}
