//! Layer identity must be shared across sheets, while first declaration fixes its
//! ordering. A tree preserves that ordering even when later sheets reopen a layer
//! or introduce another sublayer beneath an already registered parent.
use crate::parser::{LayerRule, Rule, Stylesheet};

#[derive(Default)]
struct Branch {
    children: Vec<(String, Branch)>,
}
impl Branch {
    fn register(&mut self, path: &str) {
        if path.split('.').count() > 128 {
            return;
        }
        let mut node = self;
        for part in path.split('.') {
            let index = if let Some(index) = node.children.iter().position(|(name, _)| name == part)
            {
                index
            } else {
                node.children.push((part.into(), Branch::default()));
                node.children.len() - 1
            };
            node = &mut node.children[index].1;
        }
    }
    fn flatten(&self, parent: &str, out: &mut Vec<String>) {
        for (name, branch) in &self.children {
            let path = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}.{name}")
            };
            // Direct declarations in a layer outrank its nested layers for normal
            // rules, so the parent must follow its descendants in the rank list.
            branch.flatten(&path, out);
            out.push(path);
        }
    }
}
pub(crate) fn path(parent: &str, layer: &LayerRule) -> String {
    let name = layer
        .name
        .clone()
        .unwrap_or_else(|| format!("@{:p}", layer));
    if parent.is_empty() {
        name
    } else {
        format!("{parent}.{name}")
    }
}
pub(crate) fn order(sheets: &[&Stylesheet]) -> Vec<String> {
    fn collect(rules: &[Rule], parent: &str, tree: &mut Branch) {
        for rule in rules {
            match rule {
                Rule::Layer(layer) => {
                    let path = path(parent, layer);
                    tree.register(&path);
                    collect(&layer.rules, &path, tree);
                }
                Rule::LayerOrder(names) => {
                    for name in names {
                        tree.register(&if parent.is_empty() {
                            name.clone()
                        } else {
                            format!("{parent}.{name}")
                        });
                    }
                }
                Rule::Container(rule) => collect(&rule.rules, parent, tree),
                Rule::Media(rule) => collect(&rule.rules, parent, tree),
                Rule::Supports(rule) => collect(&rule.rules, parent, tree),
                _ => {}
            }
        }
    }
    let mut tree = Branch::default();
    for sheet in sheets {
        collect(&sheet.rules, "", &mut tree);
    }
    let mut out = Vec::new();
    tree.flatten("", &mut out);
    out
}
