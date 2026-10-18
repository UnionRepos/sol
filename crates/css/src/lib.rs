//! CSS syntax parsing, selectors, cascade, and computed style.
//!
//! Use [`syntax`] when source must retain unknown rules or uninterpreted values.
//! [`parse_stylesheet`] produces the rendering adapter; syntax preservation and
//! renderer feature support are deliberately separate contracts.

pub mod conditions;
mod matching;
mod math;
pub mod parser;
pub mod selector;
pub mod style;
pub mod syntax;
pub mod ua;
pub mod values;
mod variables;

pub use parser::{parse as parse_stylesheet, parse_decls, Decl, Rule, StyleRule, Stylesheet};
pub use selector::{parse_selector, parse_selector_list, Combinator, Selector};
pub use style::{
    compute, is_known_property, match_element, style_document, Clear, Display, Float, MatchedDecl,
    NoState, Position, PseudoState, Style,
};
pub use ua::UA_CSS;
pub use values::{parse_value, value1, Value};
