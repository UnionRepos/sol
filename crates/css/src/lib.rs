//! sol_css: CSS2.1+subset parser, selectors, cascade, computed style.

pub mod parser;
pub mod selector;
pub mod style;
pub mod ua;
pub mod values;

pub use parser::{parse as parse_stylesheet, parse_decls, Decl, Rule, StyleRule, Stylesheet};
pub use selector::{parse_selector, parse_selector_list, Combinator, Selector};
pub use style::{
    compute, is_known_property, match_element, style_document, Clear, Display, Float, MatchedDecl,
    NoState, Position, PseudoState, Style,
};
pub use ua::UA_CSS;
pub use values::{parse_value, value1, Value};
