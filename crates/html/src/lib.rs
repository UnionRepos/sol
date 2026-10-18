//! sol_html: HTML5 tokenizer + tree builder.
//!
//! `parse` builds a full document; `parse_fragment` powers innerHTML.

pub mod builder;
pub mod entities;
pub mod tokenizer;

pub use builder::{parse, parse_fragment, take_fragment_children, Parsed};
pub use tokenizer::{RawKind, Token, Tokenizer};
