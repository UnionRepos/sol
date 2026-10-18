use sol_css::style::{compute, match_element, viewport, NoState};
use sol_css::{parse_selector, parse_stylesheet, parse_value, Value};
use sol_dom::Document;
use sol_html::parse;

fn styled(html: &str, css: &str, tag: &str) -> sol_css::Style {
    let mut doc = Document::new();
    parse(html, &mut doc);
    let ua = parse_stylesheet(sol_css::UA_CSS);
    let user = parse_stylesheet(css);
    let el = find(&doc, doc.root, tag).unwrap();
    let parent_el = doc.parent(el).filter(|&p| doc.node(p).is_element());
    let parent_style = parent_el.map(|p| {
        let m = match_element(&doc, p, &[&ua, &user], &NoState);
        compute(&doc, p, m, None)
    });
    let matched = match_element(&doc, el, &[&ua, &user], &NoState);
    compute(&doc, el, matched, parent_style.as_ref())
}

fn find(doc: &Document, id: sol_dom::NodeId, tag: &str) -> Option<sol_dom::NodeId> {
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
fn parse_basic_rule() {
    let s = parse_stylesheet("p { color: red; margin: 1px 2px }");
    assert_eq!(s.rules.len(), 1);
    match &s.rules[0] {
        sol_css::Rule::Style(r) => {
            assert_eq!(r.selectors.len(), 1);
            assert_eq!(r.decls.len(), 2);
            assert_eq!(r.decls[0].name, "color");
        }
        _ => panic!(),
    }
}

#[test]
fn selector_parsing() {
    let s = parse_selector("div.a > p#b + .c").unwrap();
    assert_eq!(s.parts.len(), 3);
    // subject-first: parts[0] is .c
    assert_eq!(s.parts[0].classes, vec!["c"]);
    assert_eq!(s.parts[1].id.as_deref(), Some("b"));
    assert_eq!(s.parts[2].tag.as_deref(), Some("div"));
}

#[test]
fn cascade_specificity() {
    let st = styled(
        "<p id=x class=y>t</p>",
        "p { color: red } .y { color: green } #x { color: blue }",
        "p",
    );
    assert_eq!(st.color("color"), Some([0, 0, 255, 255]));
}

#[test]
fn cascade_important() {
    let st = styled(
        "<p id=x>t</p>",
        "#x { color: red !important } p { color: blue }",
        "p",
    );
    assert_eq!(st.color("color"), Some([255, 0, 0, 255]));
}

#[test]
fn inline_style_wins() {
    let st = styled(
        "<p id=x style=\"color: orange\">t</p>",
        "#x { color: blue }",
        "p",
    );
    assert_eq!(st.color("color"), Some([255, 165, 0, 255]));
}

#[test]
fn shorthand_margin() {
    let st = styled("<div>m</div>", "div { margin: 1px 2px 3px 4px }", "div");
    assert_eq!(st.length_px("margin-top", 16.0, 800.0), Some(1.0));
    assert_eq!(st.length_px("margin-right", 16.0, 800.0), Some(2.0));
    assert_eq!(st.length_px("margin-bottom", 16.0, 800.0), Some(3.0));
    assert_eq!(st.length_px("margin-left", 16.0, 800.0), Some(4.0));
}

#[test]
fn inherited_color() {
    let mut doc = Document::new();
    parse("<div><span>x</span></div>", &mut doc);
    let ua = parse_stylesheet(sol_css::UA_CSS);
    let user = parse_stylesheet("div { color: #123456 }");
    let div = find(&doc, doc.root, "div").unwrap();
    let span = find(&doc, doc.root, "span").unwrap();
    let dm = match_element(&doc, div, &[&ua, &user], &NoState);
    let dstyle = compute(&doc, div, dm, None);
    let sm = match_element(&doc, span, &[&ua, &user], &NoState);
    let sstyle = compute(&doc, span, sm, Some(&dstyle));
    assert_eq!(sstyle.color("color"), Some([0x12, 0x34, 0x56, 255]));
}

#[test]
fn media_query() {
    viewport::WIDTH.with(|w| w.set(500.0));
    let st = styled(
        "<p>x</p>",
        "@media (max-width: 600px) { p { color: red } } @media (min-width: 800px) { p { color: blue } }",
        "p",
    );
    assert_eq!(st.color("color"), Some([255, 0, 0, 255]));
    viewport::WIDTH.with(|w| w.set(800.0));
}

#[test]
fn nth_child_sel() {
    let st = styled(
        "<ul><li>a<li>b<li>c</ul>",
        "li:nth-child(2) { color: red }",
        "li",
    );
    // first li — no match
    assert_eq!(st.color("color"), None);
}

#[test]
fn attr_selector() {
    let st = styled(
        "<a href=\"https://x.com\">l</a>",
        "a[href^=\"https\"] { color: green } ",
        "a",
    );
    assert_eq!(st.color("color"), Some([0, 128, 0, 255]));
}

#[test]
fn value_parsing() {
    let v = parse_value("12px solid #ff0000");
    assert!(matches!(v[0], Value::Length(_, _)));
    assert!(matches!(v[2], Value::Color(255, 0, 0, 255)));
}

#[test]
fn ua_stylesheet_works() {
    let st = styled("<h1>t</h1>", "", "h1");
    assert_eq!(st.display(), sol_css::Display::Block);
    // h1 is bold + 2em via UA sheet
    let fs = st.length_px("font-size", 16.0, 800.0);
    assert_eq!(fs, Some(32.0));
}

#[test]
fn elliptical_radius_shorthand_expands_and_longhands_override() {
    let style = styled(
        "<div>x</div>",
        "div{border-radius:10px 20px 30px / 5px 6px;border-top-left-radius:2px}",
        "div",
    );
    assert_eq!(
        style.get("border-top-left-radius").unwrap(),
        &sol_css::parse_value("2px")
    );
    assert_eq!(
        style.get("border-top-right-radius").unwrap(),
        &sol_css::parse_value("20px 6px")
    );
    assert_eq!(
        style.get("border-bottom-right-radius").unwrap(),
        &sol_css::parse_value("30px 5px")
    );
    assert_eq!(
        style.get("border-bottom-left-radius").unwrap(),
        &sol_css::parse_value("20px 6px")
    );
}
