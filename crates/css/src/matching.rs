//! Matching shares selector structure with parsing so invalid alternatives
//! cannot silently acquire a different specificity or ancestor relationship.
use crate::selector::{AttrOp, Combinator, Compound, Pseudo, RelativeSelector, Selector};
use crate::style::PseudoState;
use sol_dom::{Document, NodeData, NodeId};

/// Selector match — right-to-left.
pub fn matches(doc: &Document, el: NodeId, sel: &Selector, state: &dyn PseudoState) -> bool {
    matches_scoped(doc, el, sel, state, doc.document_element())
}

pub fn matches_scoped(
    doc: &Document,
    el: NodeId,
    sel: &Selector,
    state: &dyn PseudoState,
    scope: Option<NodeId>,
) -> bool {
    !sel.parts.is_empty()
        && sel.combinators.len() + 1 == sel.parts.len()
        && match_chain(doc, el, sel, 0, state, scope)
}

fn match_chain(
    doc: &Document,
    el: NodeId,
    sel: &Selector,
    i: usize,
    state: &dyn PseudoState,
    scope: Option<NodeId>,
) -> bool {
    if !match_compound(doc, el, &sel.parts[i], state, scope) {
        return false;
    }
    if i + 1 >= sel.parts.len() {
        return true;
    }
    let comb = sel.combinators[i];
    match comb {
        Combinator::Descendant => {
            let mut p = doc.parent(el);
            while let Some(n) = p {
                if doc.node(n).is_element() && match_chain(doc, n, sel, i + 1, state, scope) {
                    return true;
                }
                p = doc.parent(n);
            }
            false
        }
        Combinator::Child => match doc.parent(el) {
            Some(p) if doc.node(p).is_element() => match_chain(doc, p, sel, i + 1, state, scope),
            _ => false,
        },
        Combinator::NextSibling => {
            let sib = prev_elem_sibling(doc, el);
            match sib {
                Some(s) => match_chain(doc, s, sel, i + 1, state, scope),
                None => false,
            }
        }
        Combinator::LaterSibling => {
            let mut sib = prev_elem_sibling(doc, el);
            while let Some(s) = sib {
                if match_chain(doc, s, sel, i + 1, state, scope) {
                    return true;
                }
                sib = prev_elem_sibling(doc, s);
            }
            false
        }
    }
}

fn prev_elem_sibling(doc: &Document, n: NodeId) -> Option<NodeId> {
    let p = doc.parent(n)?;
    let kids = doc.children(p);
    let i = kids.iter().position(|&k| k == n)?;
    kids[..i]
        .iter()
        .rev()
        .copied()
        .find(|&k| doc.node(k).is_element())
}

fn elem_index(doc: &Document, n: NodeId, of_type: bool) -> usize {
    let Some(p) = doc.parent(n) else { return 1 };
    let kids = doc.children(p);
    let tag = doc.tag(n).to_string();
    let mut i = 0;
    for &k in kids {
        if !doc.node(k).is_element() {
            continue;
        }
        if of_type && doc.tag(k) != tag {
            continue;
        }
        i += 1;
        if k == n {
            return i;
        }
    }
    i
}

fn nth_match(arg: &crate::selector::NthArg, idx: usize) -> bool {
    use crate::selector::NthArg::*;
    let i = idx as i64;
    match arg {
        Odd => i % 2 == 1,
        Even => i % 2 == 0,
        N(n) => i == i64::from(*n),
        AnB(a, b) => {
            if *a == 0 {
                i == i64::from(*b)
            } else {
                let d = i - i64::from(*b);
                d % i64::from(*a) == 0 && d / i64::from(*a) >= 0
            }
        }
    }
}

fn count_following(doc: &Document, n: NodeId, of_type: bool) -> usize {
    let Some(p) = doc.parent(n) else { return 0 };
    let kids = doc.children(p);
    let tag = doc.tag(n).to_string();
    let pos = kids.iter().position(|&k| k == n).unwrap_or(0);
    kids[pos + 1..]
        .iter()
        .filter(|&&k| doc.node(k).is_element() && (!of_type || doc.tag(k) == tag))
        .count()
}

fn match_compound(
    doc: &Document,
    el: NodeId,
    c: &Compound,
    state: &dyn PseudoState,
    scope: Option<NodeId>,
) -> bool {
    let Some(e) = doc.node(el).element() else {
        return false;
    };
    if let Some(namespace) = &c.namespace {
        let uri = match e.namespace {
            sol_dom::Namespace::Html => "http://www.w3.org/1999/xhtml",
            sol_dom::Namespace::Svg => "http://www.w3.org/2000/svg",
            sol_dom::Namespace::MathMl => "http://www.w3.org/1998/Math/MathML",
        };
        if namespace != "*" && namespace != uri {
            return false;
        }
    }
    if let Some(t) = &c.tag {
        if t != "*"
            && if e.namespace == sol_dom::Namespace::Html {
                !e.tag.eq_ignore_ascii_case(t)
            } else {
                e.tag.as_ref() != t.as_str()
            }
        {
            return false;
        }
    }
    if let Some(id) = &c.id {
        if e.id() != Some(id.as_str()) {
            return false;
        }
    }
    if c.additional_ids
        .iter()
        .any(|id| e.id() != Some(id.as_str()))
    {
        return false;
    }
    for cl in &c.classes {
        let has = e
            .attr("class")
            .map(|v| v.split_ascii_whitespace().any(|x| x == cl))
            .unwrap_or(false);
        if !has {
            return false;
        }
    }
    for at in &c.attrs {
        let value = if at.namespace.as_deref() == Some("*") {
            e.attrs
                .iter()
                .find(|(name, _)| name.rsplit(':').next() == Some(&at.name))
                .map(|(_, value)| value.as_str())
        } else if let Some(namespace) = at.namespace.as_deref().filter(|ns| !ns.is_empty()) {
            match namespace {
                "http://www.w3.org/1999/xlink" => e.attr(&format!("xlink:{}", at.name)),
                "http://www.w3.org/XML/1998/namespace" => e.attr(&format!("xml:{}", at.name)),
                _ => None,
            }
        } else {
            e.attr(&if e.namespace == sol_dom::Namespace::Html {
                at.name.to_ascii_lowercase()
            } else {
                at.name.clone()
            })
        };
        let Some(value) = value else { return false };
        let (value, expected) = if at.case == Some(false) {
            (value.to_ascii_lowercase(), at.value.to_ascii_lowercase())
        } else {
            (value.to_string(), at.value.clone())
        };
        let ok = match at.op {
            AttrOp::Exists => true,
            AttrOp::Eq => value == expected,
            AttrOp::Includes => {
                !expected.is_empty() && value.split_ascii_whitespace().any(|word| word == expected)
            }
            AttrOp::DashMatch => value == expected || value.starts_with(&format!("{expected}-")),
            AttrOp::Prefix => !expected.is_empty() && value.starts_with(&expected),
            AttrOp::Suffix => !expected.is_empty() && value.ends_with(&expected),
            AttrOp::Substr => !expected.is_empty() && value.contains(&expected),
        };
        if !ok {
            return false;
        }
    }
    for p in &c.pseudos {
        let ok = match p {
            Pseudo::FirstChild => elem_index(doc, el, false) == 1,
            Pseudo::LastChild => count_following(doc, el, false) == 0,
            Pseudo::OnlyChild => {
                elem_index(doc, el, false) == 1 && count_following(doc, el, false) == 0
            }
            Pseudo::FirstOfType => elem_index(doc, el, true) == 1,
            Pseudo::LastOfType => count_following(doc, el, true) == 0,
            Pseudo::OnlyOfType => {
                elem_index(doc, el, true) == 1 && count_following(doc, el, true) == 0
            }
            Pseudo::NthChild(a) => nth_match(a, elem_index(doc, el, false)),
            Pseudo::NthOfType(a) => nth_match(a, elem_index(doc, el, true)),
            Pseudo::NthLastChild(a) => nth_match(a, count_following(doc, el, false) + 1),
            Pseudo::Empty => doc.children(el).iter().all(|&k| match &doc.node(k).data {
                NodeData::Element(_) => false,
                NodeData::Text(t) => t.is_empty(),
                _ => true,
            }),
            Pseudo::Root => doc.parent(el).is_none_or(|p| !doc.node(p).is_element()),
            Pseudo::Not(s) => !matches_scoped(doc, el, s, state, scope),
            Pseudo::Is(s) => matches_scoped(doc, el, s, state, scope),
            Pseudo::Has(s) => doc
                .descendants(el)
                .any(|d| doc.node(d).is_element() && matches(doc, d, s, state)),
            Pseudo::Hover => state.is_hovered(el),
            Pseudo::Active => state.is_active(el),
            Pseudo::Focus => state.is_focused(el),
            Pseudo::Link => &*e.tag == "a" && e.attr("href").is_some(),
            Pseudo::Visited => {
                &*e.tag == "a" && e.attr("href").map(|h| state.is_visited(h)).unwrap_or(false)
            }
            Pseudo::Checked => e.attr("checked").is_some() || e.attr("selected").is_some(),
            Pseudo::Disabled => e.attr("disabled").is_some(),
            Pseudo::Enabled => {
                matches!(
                    &*e.tag,
                    "input" | "button" | "select" | "textarea" | "option"
                ) && e.attr("disabled").is_none()
            }
            Pseudo::Target => state.is_target(el),
            Pseudo::NotList(selectors) => !selectors
                .iter()
                .any(|s| matches_scoped(doc, el, s, state, scope)),
            Pseudo::IsList(selectors) | Pseudo::Where(selectors) => selectors
                .iter()
                .any(|s| matches_scoped(doc, el, s, state, scope)),
            Pseudo::HasList(selectors) => selectors
                .iter()
                .any(|relative| relative_match(doc, el, relative, state)),
            Pseudo::NthLastOfType(nth) => nth_match(nth, count_following(doc, el, true) + 1),
            Pseudo::NthChildOf(nth, selectors) => {
                filtered_index(doc, el, selectors, state, scope, false)
                    .is_some_and(|i| nth_match(nth, i))
            }
            Pseudo::NthLastChildOf(nth, selectors) => {
                filtered_index(doc, el, selectors, state, scope, true)
                    .is_some_and(|i| nth_match(nth, i))
            }
            Pseudo::Scope => scope == Some(el),
            Pseudo::Lang(languages) => {
                inherited_attribute(doc, el, "lang").is_some_and(|language| {
                    languages
                        .iter()
                        .any(|range| language_match(&language, range))
                })
            }
            Pseudo::Dir(direction) => inherited_attribute(doc, el, "dir")
                .unwrap_or_else(|| "ltr".into())
                .eq_ignore_ascii_case(direction),
            Pseudo::AnyLink => matches!(e.tag.as_ref(), "a" | "area") && e.attr("href").is_some(),
            Pseudo::Required => {
                matches!(e.tag.as_ref(), "input" | "select" | "textarea")
                    && e.attr("required").is_some()
            }
            Pseudo::Optional => {
                matches!(e.tag.as_ref(), "input" | "select" | "textarea")
                    && e.attr("required").is_none()
            }
            Pseudo::ReadWrite => editable(doc, el),
            Pseudo::ReadOnly => !editable(doc, el),
            Pseudo::PseudoElement(_) | Pseudo::Unknown(_) => false,
        };
        if !ok {
            return false;
        }
    }
    true
}

fn relative_match(
    doc: &Document,
    el: NodeId,
    relative: &RelativeSelector,
    state: &dyn PseudoState,
) -> bool {
    let mut selector = relative.selector.clone();
    selector.parts.push(Compound {
        tag: None,
        id: None,
        classes: Vec::new(),
        attrs: Vec::new(),
        pseudos: vec![Pseudo::Scope],
        additional_ids: Vec::new(),
        namespace: None,
    });
    selector.combinators.push(relative.leading);
    let root = if matches!(
        relative.leading,
        Combinator::NextSibling | Combinator::LaterSibling
    ) {
        doc.parent(el).unwrap_or(el)
    } else {
        el
    };
    doc.descendants(root).any(|candidate| {
        doc.node(candidate).is_element()
            && matches_scoped(doc, candidate, &selector, state, Some(el))
    })
}
fn filtered_index(
    doc: &Document,
    el: NodeId,
    selectors: &[Selector],
    state: &dyn PseudoState,
    scope: Option<NodeId>,
    reverse: bool,
) -> Option<usize> {
    let parent = doc.parent(el)?;
    let mut siblings: Vec<_> = doc
        .children(parent)
        .iter()
        .copied()
        .filter(|&n| {
            doc.node(n).is_element()
                && selectors
                    .iter()
                    .any(|s| matches_scoped(doc, n, s, state, scope))
        })
        .collect();
    if reverse {
        siblings.reverse();
    }
    siblings.iter().position(|&n| n == el).map(|i| i + 1)
}
fn inherited_attribute(doc: &Document, el: NodeId, name: &str) -> Option<String> {
    let mut current = Some(el);
    while let Some(node) = current {
        if let Some(value) = doc.attr(node, name) {
            return Some(value.to_ascii_lowercase());
        }
        current = doc.parent(node);
    }
    None
}
fn language_match(language: &str, range: &str) -> bool {
    if range == "*" {
        return !language.is_empty();
    }
    let language = language.to_ascii_lowercase();
    language == range || language.starts_with(&format!("{range}-"))
}
fn editable(doc: &Document, el: NodeId) -> bool {
    let tag = doc.tag(el);
    if matches!(tag, "input" | "textarea") {
        return doc.attr(el, "readonly").is_none() && doc.attr(el, "disabled").is_none();
    }
    inherited_attribute(doc, el, "contenteditable")
        .is_some_and(|v| matches!(v.as_str(), "" | "true" | "plaintext-only"))
}
