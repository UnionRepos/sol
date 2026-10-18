use sol_dom::{Document, NodeData, NodeId};
use sol_html::{parse, parse_fragment, Token, Tokenizer};

fn text_of(doc: &Document, id: NodeId) -> String {
    doc.text_content(id)
}

fn kids(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.children(id).to_vec()
}

fn find(doc: &Document, id: NodeId, tag: &str) -> Option<NodeId> {
    if doc.is_tag(id, tag) {
        return Some(id);
    }
    for &c in doc.children(id) {
        if let Some(f) = find(doc, c, tag) {
            return Some(f);
        }
    }
    None
}

#[test]
fn tokenizer_basic() {
    let mut t = Tokenizer::new("<p class=\"x\">hello</p>");
    match t.next_token() {
        Token::StartTag { name, attrs, .. } => {
            assert_eq!(name, "p");
            assert_eq!(attrs, vec![("class".to_string(), "x".to_string())]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(t.next_token(), Token::Text("hello".into()));
    assert_eq!(t.next_token(), Token::EndTag { name: "p".into() });
    assert_eq!(t.next_token(), Token::Eof);
}

#[test]
fn tokenizer_entities() {
    let mut t = Tokenizer::new("&lt;a&gt;&amp; &#65;&#x42; &nbsp;");
    assert_eq!(t.next_token(), Token::Text("<a>& AB \u{a0}".into()));
}

#[test]
fn tokenizer_comment_doctype() {
    let mut t = Tokenizer::new("<!doctype html><!-- hi --><b>");
    match t.next_token() {
        Token::Doctype { name, .. } => assert_eq!(name, "html"),
        other => panic!("{other:?}"),
    }
    assert_eq!(t.next_token(), Token::Comment(" hi ".into()));
    assert!(matches!(t.next_token(), Token::StartTag { name, .. } if name == "b"));
}

#[test]
fn build_simple_doc() {
    let mut doc = Document::new();
    parse(
        "<!doctype html><html><head><title>T</title></head><body><p id=a>hi</p></body></html>",
        &mut doc,
    );
    assert!(!doc.quirks_mode);
    let root = doc.root;
    let title = find(&doc, root, "title").unwrap();
    assert_eq!(text_of(&doc, title), "T");
    let p = find(&doc, root, "p").unwrap();
    assert_eq!(doc.attr(p, "id"), Some("a"));
    assert_eq!(text_of(&doc, p), "hi");
}

#[test]
fn implied_body_and_p_close() {
    let mut doc = Document::new();
    // No html/body tags at all; <p> closed implicitly by <div>
    parse("<p>one<div>two</div>", &mut doc);
    let root = doc.root;
    let p = find(&doc, root, "p").unwrap();
    let div = find(&doc, root, "div").unwrap();
    assert_eq!(text_of(&doc, p), "one");
    assert_eq!(text_of(&doc, div), "two");
    // div is not inside p
    assert!(doc.ancestors(div).all(|a| !doc.is_tag(a, "p")));
}

#[test]
fn list_items_auto_close() {
    let mut doc = Document::new();
    parse("<ul><li>a<li>b<li>c</ul>", &mut doc);
    let ul = find(&doc, doc.root, "ul").unwrap();
    let lis = kids(&doc, ul);
    assert_eq!(lis.len(), 3);
    assert_eq!(text_of(&doc, lis[2]), "c");
}

#[test]
fn table_structure() {
    let mut doc = Document::new();
    parse("<table><tr><td>1<td>2<tr><td>3</table>", &mut doc);
    let tds = {
        let mut v = Vec::new();
        collect_tag(&doc, doc.root, "td", &mut v);
        v
    };
    assert_eq!(tds.len(), 3);
    assert_eq!(text_of(&doc, tds[0]), "1");
    assert_eq!(text_of(&doc, tds[2]), "3");
}

fn collect_tag(doc: &Document, id: NodeId, tag: &str, out: &mut Vec<NodeId>) {
    if doc.is_tag(id, tag) {
        out.push(id);
    }
    for &c in doc.children(id) {
        collect_tag(doc, c, tag, out);
    }
}

#[test]
fn foster_parenting() {
    let mut doc = Document::new();
    // "text" inside table but outside cells gets fostered before the table
    parse("<div><table>stray<td></table></div>", &mut doc);
    let div = find(&doc, doc.root, "div").unwrap();
    let table = find(&doc, div, "table").unwrap();
    // "stray" should be a text sibling before <table>
    let dk = kids(&doc, div);
    let ti = dk.iter().position(|&k| k == table).unwrap();
    assert!(ti > 0);
    let before = &dk[..ti];
    let txt: String = before
        .iter()
        .filter_map(|&n| match &doc.node(n).data {
            NodeData::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect();
    assert!(txt.contains("stray"));
}

#[test]
fn adoption_agency() {
    let mut doc = Document::new();
    // classic mis-nesting: <b>bold <i>both</b> italic</i>
    parse("<b>bold <i>both</b> italic</i>", &mut doc);
    let b = find(&doc, doc.root, "b").unwrap();
    let txt_b = text_of(&doc, b);
    assert!(txt_b.contains("bold"));
    assert!(txt_b.contains("both"));
    // an <i> should exist outside <b> containing " italic" (via clone)
    let mut is = Vec::new();
    collect_tag(&doc, doc.root, "i", &mut is);
    let all_text: String = is.iter().map(|&i| text_of(&doc, i)).collect();
    assert!(all_text.contains("both"));
    assert!(all_text.contains("italic"));
}

#[test]
fn script_raw_text() {
    let mut doc = Document::new();
    let p = parse("<script>if (a<b) { }</script><p>x", &mut doc);
    assert_eq!(p.scripts.len(), 1);
    let s = p.scripts[0];
    let raw: String = doc
        .children(s)
        .iter()
        .filter_map(|&n| match &doc.node(n).data {
            NodeData::Text(t) => Some(t.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(raw, "if (a<b) { }");
}

#[test]
fn entities_in_attr() {
    let mut doc = Document::new();
    parse("<a href=\"?a=1&amp;b=2\">x</a>", &mut doc);
    let a = find(&doc, doc.root, "a").unwrap();
    assert_eq!(doc.attr(a, "href"), Some("?a=1&b=2"));
}

#[test]
fn select_options() {
    let mut doc = Document::new();
    parse(
        "<select><option>a<option>b<optgroup><option>c</select>",
        &mut doc,
    );
    let sel = find(&doc, doc.root, "select").unwrap();
    let mut opts = Vec::new();
    collect_tag(&doc, sel, "option", &mut opts);
    assert_eq!(opts.len(), 3);
    let mut og = Vec::new();
    collect_tag(&doc, sel, "optgroup", &mut og);
    assert_eq!(og.len(), 1);
}

#[test]
fn fragment_parse() {
    let mut doc = Document::new();
    let nodes = parse_fragment("<li>one<li>two", "ul", &mut doc);
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|&n| doc.is_tag(n, "li")));
    assert_eq!(text_of(&doc, nodes[1]), "two");
}

#[test]
fn fragment_table_row() {
    let mut doc = Document::new();
    let nodes = parse_fragment("<td>cell1<td>cell2", "tr", &mut doc);
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|&n| doc.is_tag(n, "td")));
}

#[test]
fn nested_formatting() {
    let mut doc = Document::new();
    parse("<div><b><u>deep</u></b> tail</div>", &mut doc);
    let b = find(&doc, doc.root, "b").unwrap();
    let u = find(&doc, doc.root, "u").unwrap();
    assert_eq!(text_of(&doc, u), "deep");
    assert_eq!(text_of(&doc, b), "deep");
    let div = find(&doc, doc.root, "div").unwrap();
    assert_eq!(text_of(&doc, div), "deep tail");
}

#[test]
fn textarea_rcdata() {
    let mut doc = Document::new();
    parse("<textarea><b>not bold</b>&amp;</textarea>", &mut doc);
    let ta = find(&doc, doc.root, "textarea").unwrap();
    assert_eq!(text_of(&doc, ta), "<b>not bold</b>&");
}

#[test]
fn head_elements_stay_in_head() {
    let mut doc = Document::new();
    parse(
        "<html><head><meta charset=utf-8><link href=x></head><body></body></html>",
        &mut doc,
    );
    let head = find(&doc, doc.root, "head").unwrap();
    let body = find(&doc, doc.root, "body").unwrap();
    assert_eq!(kids(&doc, head).len(), 2);
    assert!(kids(&doc, body).is_empty());
}
