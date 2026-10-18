# CSS parsing and style evaluation

[Project overview](../README.md) · [Architecture](architecture.md) ·
[Development](development.md) · [Limits](limits.md)

`sol_css` separates general CSS syntax from selectors, property interpretation,
and rendering. The syntax layer follows the token and balanced-component model
of [CSS Syntax Level 3](https://www.w3.org/TR/css-syntax-3/). It can retain rules
and values from newer CSS modules without claiming that layout implements them.
The regression suite exercises these contracts; it is not a complete WPT
conformance run.

## Choose an API

| API | Intended use | Result |
|---|---|---|
| `syntax::tokenize` | Inspect decoded input, escapes, numbers, URLs, and token locations | Normalized source, tokens, recoverable errors |
| `syntax::tokenize_with_unicode_ranges(input, true)` | Tokenize a descriptor context that recognizes Unicode ranges | Same result, with `UnicodeRange` tokens |
| `syntax::parse_component_values` | Retain arbitrary values, functions, and balanced blocks | Component values and errors |
| `syntax::parse_component_value` | Require exactly one component | `Result<ComponentValue, Vec<ParseError>>` |
| `syntax::parse_stylesheet` | Preserve general qualified rules and arbitrary at-rules | Syntax AST and errors |
| `syntax::parse_rule` | Require one rule without trailing non-whitespace content | `Result<syntax::Rule, Vec<ParseError>>` |
| `syntax::parse_declaration` | Parse one syntactic declaration | Name, raw component values, `important` |
| `syntax::parse_block_contents` | Inspect mixed declarations, nested rules, and at-rules in source order | Ordered `BlockItem`s and errors |
| `parse_stylesheet` | Build styles for the renderer | Typed style, grouping, layer, container, keyframe, registration, font-face, and import rules; other at-rules preserved |
| `parse_decls` | Parse inline declarations for the style engine | Typed declarations, with invalid declarations skipped |
| `parse_selector`, `parse_selector_list` | Parse a selector or strict selector list | Subject-first selectors with specificity |
| `parse_value` | Interpret common colors, dimensions, functions, and values | Typed values; unknown dimensions/functions remain represented |

Input is an already decoded Rust `&str`; transport character encoding is the
caller's responsibility. CRLF, CR, and form feed normalize to LF; null characters
become U+FFFD. Token offsets and diagnostic offsets are UTF-8 byte offsets into
this **normalized** source, not the original network bytes. Tokens have start and
end offsets. Grouped functions and blocks do not currently carry their own spans.

```rust
use sol_css::syntax::{self, Rule};

let sheet = syntax::parse_stylesheet(
    "@container (width > 20rem) { .card { --theme: { accent: blue }; } }",
);
assert!(sheet.errors.is_empty());
let Rule::At(container) = &sheet.rules[0] else { panic!("expected at-rule") };
assert_eq!(container.name, "container");
assert!(container.block.is_some());
let reparsed = syntax::parse_rule(&sheet.rules[0].to_css()).unwrap();
assert_eq!(reparsed.to_css(), sheet.rules[0].to_css());
```

## Tokenization, preservation, and recovery

Identifiers support non-ASCII characters, escaped punctuation, and one-to-six
hexadecimal digit escapes with optional trailing whitespace. Invalid escaped
code points become U+FFFD. Numeric tokens retain their spelling and integer flag;
signs, fractions, and scientific notation use CSS lookahead. Unknown dimensions
retain their unit instead of being converted to pixels.

Strings, raw URL tokens, quoted `url()` functions, comments, hash ID flags,
CDO/CDC, and punctuation have distinct token forms. Delimiters inside strings or
balanced functions cannot terminate an enclosing declaration. A bad string or URL
is recorded and consumption resumes at its CSS recovery boundary. EOF closes
open component blocks with diagnostics; an unterminated comment ends at EOF.
A qualified rule without a block is discarded, while an at-rule may end at EOF.

The generic AST retains arbitrary at-rule names, preludes, and blocks. Use
`parse_rule_list` to interpret an at-rule's block as child rules, or
`parse_block_contents` to interpret a textual declaration block. Their context
is selected explicitly: a keyframe block, declaration block, and nested style
block do not have interchangeable grammars.

`to_css()` and `syntax::serialize()` produce canonical CSS, not source text.
Comments, original whitespace, quote style, and escape spellings are not retained.
Separating components with comments prevents accidental token merging without
introducing selector descendant whitespace. Canonical serialization preserves
valid token boundaries, including exponent-looking dimension units. Error tokens
are diagnostic artifacts; exact recovery-token round trips are not guaranteed.

Ordinary declaration names are ASCII-lowercased; custom property names preserve
case. Trailing `!important` is recognized from tokens, including intervening
comments or whitespace, and never from quoted text. An empty ordinary declaration
is syntactically representable, but the rendering adapter discards it. Empty
custom properties remain valid. A sole curly block can form an ordinary generic
value; mixing it with other top-level components is rejected. `unicode-range`
descriptors use contextual range tokens while custom property values keep their
ordinary tokenization. Property-specific validity is a later concern.

## Selectors and matching

The selector engine supports type, universal, ID, class, attribute, descendant,
child, adjacent-sibling, and following-sibling selectors. Attribute operators
include `=`, `~=`, `|=`, `^=`, `$=`, and `*=`, with explicit ASCII `i` and `s` flags.
Empty substring operands do not match. Escapes are decoded before DOM comparison.
Namespace-qualified selector syntax is represented; stylesheet `@namespace`
prefixes resolve from a stylesheet's `@namespace` declarations; unresolved
prefixes invalidate the affected strict rule. See [rendering](rendering.md) for
resource-loading and namespace boundaries.

Structural selectors include child/type position, reverse positions, `An+B`,
and `:nth-child(... of selector-list)` / `:nth-last-child(... of selector-list)`.
`:is()` and `:where()` use forgiving lists; ordinary lists and `:not()` are strict.
`:has()` supports anchored descendant, child, and sibling relative selectors and
rejects nested `:has()` and pseudo-elements. `:where()` contributes no specificity;
`:is()`, `:not()`, and `:has()` use their most specific argument. Filtered nth
selectors add the pseudo-class specificity to their most specific filter.

`:scope`, `:lang()`, `:dir()`, link, form-state, and interactive state selectors
have implementations. Language and form matching are simplified DOM-based
approximations; direction uses inherited `dir`, rather than full Unicode
content-based directionality. Generated `::before`, `::after`, and marker text feed layout; other
pseudo-elements retain the rendering limitations in the [rendering guide](rendering.md). Unknown pseudo-classes invalidate strict selector lists.

`style::matches_scoped` accepts an explicit scope node. JavaScript `matches()`
uses its element as scope; element queries use their query root. Queries parse a
selector list once, then share it across candidates. Invalid selector queries
currently return no matches; the JS binding does not yet throw DOM SyntaxError.

## Variables, lengths, and colors

Custom properties enter the cascade with case-sensitive names and raw component
values. Inheritance copies their computed token values. Ordinary values containing
`var()` remain deferred until the element's winning custom properties are known.
Substitution supports nested fallbacks, missing variables, empty fallbacks,
cycles, and references inside functions. Fallback dependencies participate in
cycle detection even when the fallback is not selected. Substitution preserves
token boundaries: `var(--number)px` does not become a dimension.

Shorthands containing variables expand after substitution, preserving declaration
priority relative to longhands. Invalid computed substitutions use `unset`
behavior rather than resurrecting a lower-priority declaration. Validation covers
common interpreted properties; the engine does not implement every CSS module's
property grammar. Layer rollback and common `@property` syntax registrations are
implemented; the [rendering guide](rendering.md) lists their supported grammar and
remaining cascade/registration limits.

Length evaluation handles `calc()`, `min()`, `max()`, and `clamp()`, including
parentheses, arithmetic precedence, percentages, and supported absolute/font/
viewport units. Addition requires compatible dimensions and CSS whitespace around
`+` and `-`. Multiplication permits one length operand; division requires a
nonzero number. Non-length units, incompatible dimensions, and non-finite results
cannot become pixel lengths. This is length arithmetic, not the complete modern
CSS typed arithmetic system. Viewport units use the viewport, independently of
the containing block's percentage base.

Colors include named colors, 3/4/6/8-digit hex, legacy comma and modern space/slash
RGB/HSL/HWB, Lab/LCH, OKLab/OKLCH, supported RGB/XYZ `color()` profiles, and
`color-mix()` in selected spaces. Gradients remain structured functions until
painting. The [rendering guide](rendering.md) explains conversion, compositing,
interpolation, and gamut limits.

## Conditional rules and rendering boundaries

The adapter evaluates nested `@media` and `@supports`. Boolean conditions support
`not`, `and`, `or`, and parenthesized grouping. Mixing ungrouped `and` and `or`
is invalid; malformed grammar does not turn true when negated. Query-level media
`not` negates the entire medium-and-condition result.

Media evaluation includes width, height, aspect ratio, resolution, color depth,
min/max forms, and comparison/chained ranges. Orientation follows the viewport.
The current environment is static: resolution is 96 dpi, color depth 8 bits,
light color scheme, no reduced-motion preference, hover capability, and a fine
pointer. These are engine defaults, not host-device detection. `viewport::MEDIUM`
selects screen/print behavior. Unknown features evaluate false.

`@supports` checks known property/value forms and `selector(...)`, including
implemented grid display values. Its property checks are not a complete
standards-wide validation database.

The rendering adapter activates nested rules, cascade layers, size-container
queries, keyframes, and registered custom properties. Kernel loading expands
imports and activates supported font faces and background resources. Unknown
at-rules, including paged-media rules without an implemented renderer, remain
preserved. See [CSS rendering](rendering.md) for the concrete supported behavior,
API examples, budgets, and remaining browser-conformance gaps.

## Budgets and verification

| Limit | Behavior |
|---|---|
| 1,000,000 tokens per tokenization | Stop consuming and report `LimitExceeded` |
| 128 component nesting levels | Discard the overflowing balanced region and report `LimitExceeded` |
| 64 selector interpretation depth | Reject an excessively nested selector |
| 128 variable dependency/substitution depth | Treat the variable as invalid |
| 65,536 expanded components per value | Treat an oversized expansion as invalid; permit a caller's fallback |

These limits do not bound total source bytes, all computed custom properties
across a document, DOM size, or total process memory. The kernel separately caps
combined stylesheet source at 16 MiB. Callers parsing arbitrary text directly
should enforce a source-size budget appropriate to their application.

Run the focused regression suites with:

```bash
cargo test -p sol_css --offline
cargo test -p sol_kernel --test lifecycle --offline
```

[`syntax.rs`](../crates/css/tests/syntax.rs) covers lexical edge cases, generic
rules, declaration recovery, canonical serialization, depth limits, and a
reproducible 1,500-input malformed Unicode corpus.
[`modern.rs`](../crates/css/tests/modern.rs) covers selector validity/matching/
specificity, variables and expansion budgets, math, colors, media, supports,
and shorthand interpretation. Kernel lifecycle tests additionally verify DOM
selector queries and that CSS reaches geometry and raster pixels. The existing
workspace CI runs these in debug and release and requires warning-free Clippy.
