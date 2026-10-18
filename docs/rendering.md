# CSS rendering

[Project overview](../README.md) · [CSS parsing](css.md) ·
[Architecture](architecture.md) · [Development](development.md)

CSS now travels through the complete project pipeline: stylesheet discovery and
imports, selector matching and the cascade, computed styles, layout, display-list
construction, raster compositing, and hit testing. This guide describes the
implemented behavior and its limits. It does not claim complete browser or
Web Platform Tests conformance.

## Run the showcase

```bash
cargo run -p sol -- shot demo/css-rendering.html /tmp/sol-css-rendering.png 1000x640
```

[`demo/css-rendering.html`](../demo/css-rendering.html) is self-contained apart
from the repository's bundled font. It exercises nested rules in named layers,
registered custom properties, grid tracks, generated labels, gradients, rounded
shadows, opacity groups, and container queries. It does not fetch external hosts.

## Responsibilities

| Boundary | Owner | Reason for the separation |
|---|---|---|
| Imports, source URLs, font files | [`kernel::stylesheets`](../crates/kernel/src/stylesheets.rs) | Loading policy and budgets belong to the browser host |
| Background image fetching and source changes | [`kernel::resources`](../crates/kernel/src/resources.rs) | Resource identity and stylesheet ordering must survive parallel fetching |
| Nesting, namespaces, layer ordering | [`css::nesting`](../crates/css/src/nesting.rs), [`namespaces`](../crates/css/src/namespaces.rs), [`layers`](../crates/css/src/layers.rs) | Parsing structure and cascade priority must agree before matching |
| Registered custom properties | [`css::properties`](../crates/css/src/properties.rs), [`variables`](../crates/css/src/variables.rs) | Defaults and type checks must happen before consumers substitute a variable |
| Animation time and interpolation | [`css::animations`](../crates/css/src/animations.rs), [`transitions`](../crates/css/src/transitions.rs) | Sampling must be reproducible independently of network or test timing |
| Grid tracks and cell placement | [`layout::grid`](../crates/layout/src/grid.rs) | Intrinsic probes must not change later placement or leak probe boxes |
| Generated content and counters | [`layout::generated`](../crates/layout/src/generated.rs) | Generated boxes need normal layout without modifying the DOM |
| Background sampling | [`paint::background`](../crates/paint/src/background.rs) | Images and gradients must share positioning, clipping, and transforms |
| Stacking and compositing | [`paint::stacking`](../crates/paint/src/stacking.rs), [`effects`](../crates/paint/src/effects.rs) | Descendants must remain atomic when a parent creates a stacking context |
| Shadow coverage | [`paint::shadows`](../crates/paint/src/shadows.rs) | Blur must operate on coverage before compositing color |

## Stylesheets and computed styles

Linked sheets retain their document positions even when fetching overlaps.
Imports expand recursively at the import's position, with cycle, depth, count,
and aggregate source limits. Import media, supports, and layer qualifiers are
retained. URLs resolve against each response's final URL, including imports and
background/font references. Imports following ordinary style rules are ignored.
Separate top-level sheets retain their own namespace declarations.

`@font-face` can activate supported TTF/OpenType font data under its declared
family, weight, and style. Registration is page-local: it does not replace
system generic-family selection or leak into another page. Font collections
use their first face. WOFF/WOFF2 decoding, variable-font axes, complete font
matching, and `local()` sources are not implemented.

The kernel fingerprints stylesheet sources, media attributes, and viewport size.
A dirty update reparses changed `<style>` text or linked-sheet declarations;
unchanged sources reuse the existing cascade and fonts. Background resources
introduced by changed computed styles load before relayout. This does not make
all resource loading asynchronous or dynamically discover newly inserted `<img>`
elements.

Nested selectors expand using the parent's selector list through `:is()`,
preserving its maximum specificity. Ampersands inside strings remain strings.
Declarations on either side of nested rules retain source order. Grouping rules
inside nested styles retain their parent selector context.

Named and anonymous layers participate in cascade ordering. Unlayered normal
rules outrank layered rules; important layer order reverses. `revert-layer`
removes candidates from the current layer, and `revert` removes candidates from
the current origin. This is a compact UA/author/inline cascade rather than a
complete implementation of user origins, scope proximity, and shadow-tree order.

`@property` implements `inherits`, an initial value, and validation for the
supported syntax types: length, length-percentage, number, integer, percentage,
angle, time, resolution, color, custom identifier, string, URL, literals, unions,
and common `+`/`#` lists. Unsupported registration grammars remain inactive.
Validation is not the complete Properties and Values API grammar or its full
computational-independence analysis. Registered custom-property animation and
complete CSSOM registration APIs remain separate work.

`font-size` resolves relative to the parent's computed size before inheritance.
`rem` uses the document's computed root size. Rendering restores the page's own
viewport and root metrics before evaluating backgrounds, so rendering a second
page on the same thread does not change the first page's dimensions.

## Layout and generated boxes

Grid supports fixed, percentage, fractional, auto, `minmax()`, and `fit-content()`
tracks; numeric and bounded auto repetition; implicit tracks; gaps; explicit and
negative line numbers; row/column spans; named rectangular template areas;
row/column auto flow; dense placement; item order; and basic self alignment and
stretch. Definite cells reserve space before auto placement. Grid items and
anonymous text items use the same box/tree boundaries as flex items. Inline grid
and flex containers participate as inline blocks.

Track sizing uses a compact intrinsic measurement strategy. It is not the full
Grid track-sizing algorithm: subgrid, masonry, named grid lines, full intrinsic
min/max behavior, all auto-fit collapse cases, vertical writing modes, and full
content-distribution/baseline alignment are not implemented. Existing block,
inline, float, flex, table, and positioning algorithms retain their documented
approximations. Specified block widths can overflow their containing blocks;
inline-block placement translates the complete descendant subtree.

`::before` and `::after` create boxes when `content` generates something, including
an empty string. Strings, `attr()`, quotes, `counter()`, and `counters()` supply
text. The pseudo's display, font, color, dimensions, background, borders, and
other ordinary box styles feed normal layout. `::marker` can supply generated
text. Counter reset/set/increment state uses DOM scopes and supports decimal,
leading-zero, alphabetic, and Roman formatting. Quotes use declared string pairs
and nesting depth. Generated boxes do not insert synthetic DOM nodes.

Generated image content, custom counter-style definitions, complete table-counter
behavior, selection pseudo boxes, and full pseudo-element animation are not
implemented. Pseudo styles are accessible through `Style::pseudos`.

Named and unnamed size-container queries find the nearest eligible ancestor.
`container-type:inline-size` supports inline-axis size tests; `size` also permits
block-size tests. Layout feeds content-box dimensions into the next style pass.
A refresh keeps one animation time while iterating container styles/layout up to
eight times. Style queries, scroll-state queries, container units, and a complete
containment/intrinsic-sizing implementation are not provided.

## Painting, compositing, and hit testing

Background layers paint back to front above the background color. Linear,
radial, conic, and repeating gradients implement color stops and common geometry.
Image and gradient layers support position, explicit/percentage size, cover,
contain, repeat/no-repeat, repeat-x/y, round, space, origin/clip boxes, and common
attachment behavior. HTML/body backgrounds propagate to the canvas, including
empty pages. Gradient interpolation uses premultiplied sRGB; interpolation hints,
all Color 4 interpolation spaces, background-clip:text, image-set, and every
extended gradient grammar are not implemented.

Colors include the previous RGB/HSL/HWB syntax plus Lab/LCH, OKLab/OKLCH, supported
`color()` RGB/XYZ profiles, and `color-mix()` in sRGB, linear sRGB, or OKLab.
Wide-gamut results convert to the RGBA8 canvas and clip to sRGB. This is not
perceptual gamut mapping, complete missing-channel handling, relative colors,
or a wide-gamut/HDR framebuffer.

A stacking context keeps transforms, opacity groups, and explicit eligible
`z-index` subtrees atomic. Positioned descendants can escape non-stacking
ancestors. Absolutely positioned boxes retain their parent relationship, so
parent opacity and clipping still apply. Normal flow, floats, inline content,
positioned content, and negative/positive stacking levels feed an ordered list.
This is not the complete CSS painting model for every table, fragmentation,
pseudo-element, fixed-position clipping, or mixed-border case.

Opacity applies after a subtree is composited, rather than fading each child
independently. Clip coverage applies once at the group's edge. Outer/inset box
shadows and text shadows blur coverage with bounded box-filter passes. Outlines
paint without changing layout. CSS filters, blend modes, backdrop filters,
mask images, clip-path geometry, and 3D perspective are not implemented.

Image `object-fit` supports fill, contain, cover, none, and scale-down with
positioning. Image codecs remain the project's PNG/GIF/BMP implementations.

The display list also records hit candidates. `Page::hit` replays its transform
and clip state in paint order; the visible topmost eligible box wins. Rounded
clipping, `pointer-events`, and z-order therefore share paint's ordering rather
than using a separate reversed tree traversal. Opacity zero remains hittable.

## Animation and transitions

```rust
use sol_kernel::Page;
use sol_url::Url;

let mut page = Page::new(320, 200);
page.load_html(
    "<style>@keyframes grow{from{width:10px}to{width:50px}}\
     #box{height:20px;background:red;animation:grow 2s linear forwards}</style>\
     <div id=box></div>",
    &Url::parse("file:///tmp/animation.html").unwrap(),
).unwrap();
page.sample_animations(1.0);
let png = page.render_png();
assert!(png.starts_with(b"\x89PNG"));
```

Keyframes support endpoints, percentage offsets, duration/delay, iteration count,
direction, fill mode, common timing keywords, cubic Bézier curves, and steps.
Interpolation covers numbers, percentages, compatible lengths, colors, and
matching function lists. Important declarations outrank keyframe effects.
Transitions retain their target between samples, interpolate from the previous
computed value, and apply above animation effects.

`Page::settle` samples active effects using elapsed page time. Hosts drive event
turns; there is no background animation thread. `Page::sample_animations(seconds)`
selects explicit page time for tests and screenshots and refreshes style/layout.
`Page::refresh_rendering` performs the coupled style/resource/container/layout
refresh. Lower-level style callers can use `style_document_with_timeline` and a
page-owned `animations::Timeline`.

This is not the complete Web Animations platform. Event dispatch, animation
composition modes, scroll/view timelines, transition reversing semantics,
full pause/resume behavior, transform decomposition, pseudo timelines, and
complete property-specific interpolation are not implemented. Changing an
animation's configuration can restart the element's animation set.

## Resource limits and regression evidence

Existing network, image, canvas, glyph, and source budgets still apply.
Additional fixed limits are:

| Resource | Limit / behavior |
|---|---|
| Recursive imports | 16 active levels, 256 import attempts, cycle detection |
| CSS source and expanded sheet output | 16 MiB aggregate source; bounded expanded output retention |
| Nesting expansion | 65,536 components per expanded selector; 1,000,000 expansion components per parse |
| Layer hierarchy | 128 segments |
| Grid tracks | 256 tracks per axis |
| Background layers | Up to 128 layers painted |
| Animation lists / keyframe offsets | Up to 128 animations per element / 1,024 offsets per keyframe rule |
| Extra opacity buffers | 128 MiB retained; unavailable groups suppress paint |
| Shadow masks | At most 16 megapixels; blur radius capped at 128 device pixels per pass |
| Web fonts | 32 MiB per response; 64 MiB activated bytes per stylesheet load |
| Container feedback | Up to eight style/layout passes per refresh |

These are individual limits, not a process-wide memory or execution-time quota.
Oversized or unsupported constructions can be ignored, bounded, or approximated.

Tests cover grid geometry and areas; layers and rollback; nesting; registered
custom properties; namespaces; color conversion; font isolation; generated
content and counters; imported background resources; live stylesheet edits;
container resize feedback; fixed-time keyframes and transitions; grouped opacity;
paint/hit z-order; gradients, clipping, shadows, outlines, relative fonts; and
inline-block descendant placement. The original screenshot fixture remains a
byte-for-byte regression check.

The implementation is informed by [CSS Grid](https://www.w3.org/TR/css-grid-1/),
[Backgrounds and Borders](https://www.w3.org/TR/css-backgrounds-3/), and
[CSS Color](https://www.w3.org/TR/css-color-4/); passing these local tests is not
an implementation report for those standards.
