//! Query matching consumes the previous layout snapshot because style determines
//! layout while container sizes depend on layout. Keeping that snapshot explicit
//! lets the host bound feedback passes without recursing into layout from matching.
use crate::parser::ContainerRule;
use sol_dom::{Document, NodeId};
use std::{cell::RefCell, collections::HashMap};
#[derive(Clone, Debug, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
    pub names: Vec<String>,
    pub block: bool,
}
thread_local! {static BOXES:RefCell<HashMap<NodeId,Size>>=RefCell::new(HashMap::new());}
pub fn set_boxes(boxes: HashMap<NodeId, Size>) {
    BOXES.with(|current| *current.borrow_mut() = boxes);
}
pub(crate) fn matches(doc: &Document, element: NodeId, rule: &ContainerRule) -> bool {
    BOXES.with(|boxes| {
        let boxes = boxes.borrow();
        let mut parent = doc.parent(element);
        while let Some(node) = parent {
            if let Some(size) = boxes.get(&node) {
                if rule
                    .name
                    .as_ref()
                    .is_none_or(|name| size.names.contains(name))
                    && (!rule.condition.needs_block_size() || size.block)
                {
                    return rule.condition.matches_size(size.width, size.height);
                }
            }
            parent = doc.parent(node);
        }
        false
    })
}
