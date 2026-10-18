# Supported behavior, budgets, and diagnostic limits

[Project overview](../README.md) · [Architecture](architecture.md) ·
[Usage and embedding](usage.md) · [Development](development.md)

This guide describes the current implementation. Browser features are subsets,
and passing the local suite does not establish full web-platform conformance.

## Resource budgets

MiB below means 1024 × 1024 bytes. Limits belong to different owners and are not
one process-wide memory allowance.

| Resource | Default or fixed limit | Enforcement / configuration |
|---|---|---|
| HTTP exchange | 25 seconds by default | `Client::timeout`; each exchange includes its body, not the whole redirect chain |
| Decoded response body | 64 MiB by default | `Client::max_body_bytes` |
| HTTP wire-body reader | 64 MiB cap | Transport reader configuration; separate decoded checks also apply |
| Redirect loop | At most nine loop iterations | `0..=MAX_REDIRECTS`, where `MAX_REDIRECTS` is eight; a chain still redirecting after those iterations fails |
| Response headers | 65536 bytes | ureq agent configuration |
| Batch workers | 1–8, additionally limited by input size | `get_many` clamps the requested parallelism; an empty batch starts no workers |
| Page resource parallelism | Four by default | `Page::resource_parallelism`, clamped by `get_many` |
| Retained batch response bodies | 64 MiB | Includes owned duplicate-response clones; not active worker allocations or cache copies |
| HTTP cache | 128 entries and 32 MiB of bodies | LRU eviction; cache overhead is additional |
| Remaining cache freshness | At most one hour | Explicit origin freshness reduced by response age and elapsed request time |
| CSS tokens / component nesting | 1,000,000 tokens / 128 nesting levels | Syntax diagnostics; see [CSS parsing](css.md) for selector and variable limits |
| Combined author stylesheet text | 16 MiB | Kernel collection budget, including separators |
| Decoded images retained by one page | 64 MiB of shared pixels | Unique image allocations charged once; encoded responses and temporary decoder memory are additional |
| Image decode input | 64 MiB | Shared codec input validation |
| Image width and height | 1–16384 pixels each | Codec/allocation dimension validation |
| Image pixel count | 16 × 1024 × 1024 | Product checked before allocation |
| Canvas dimensions | Same side/pixel limits as above | `Canvas::validate_dimensions` / `try_new`; 64 MiB maximum framebuffer |
| Clip masks | 64 MiB retained | Exhaustion clips further content rather than growing without bound |
| Discovered font file | 32 MiB | Bounded file read during discovery |
| Discovered font data | 64 MiB accounting budget | Includes bundled fallback reservation; collection faces can each retain their own data clone |
| Faces considered per font collection | 256 | Collection discovery cap |
| Font size | 0–512 px after normalization | Finite sizes are clamped to that range; non-finite values use 16 px |
| Glyph bitmap width/height | 2048 pixels | Rasterization limit |
| Microtasks per drain | Guard at 100000 callbacks | Count guard, not a callback deadline or total realm resource bound |
| Timer rounds during loading / settling | Eight / four | Host pumping limits; no wait for all future timers |

The number of active requests and the body caps do not bound the whole process.
A page can hold a framebuffer, decoded images, cache bodies, fonts, layout state,
JavaScript objects, and in-flight response buffers simultaneously. Decode and
raster paths can require temporary buffers. There is no general DOM-size,
JavaScript-heap, source-size, execution-time, or complete process-memory quota.

Some constructors are intentionally infallible conveniences: `Image::new` and
`Canvas::new` call their fallible counterparts and panic on failure. Prefer
`try_new` at lower-level caller boundaries where allocation errors must propagate.
The `Page` rendering convenience path uses the infallible canvas constructor.

## Networking and trust boundaries

HTTP/HTTPS uses synchronous ureq exchanges and rustls with platform trust.
Certificate and hostname verification are enabled by default. There is no fallback
to the experimental TLS crate. The client can explicitly disable verification;
that setting is part of cache identity and causes the pooled agent to be rebuilt.

Redirects accept only HTTP(S), reject an HTTPS-to-HTTP downgrade, and remove
sensitive request headers across origins. POST becomes GET for 301/302, and 303
becomes GET except for HEAD. 307/308 preserve method and body. Rewritten GETs also
remove body metadata. These rules belong to the client layer rather than ureq's
automatic redirect handling.

The cookie jar supports host-only/domain scope, path boundaries, secure matching,
HttpOnly metadata, and Max-Age. It is not a full browser cookie implementation:
there is no public-suffix-list validation, SameSite policy, persistence, or
Expires-date handling in this parser. The JavaScript `document.cookie` binding
uses a separate runtime string; it is not wired to the network client's cookie
jar, including that jar's HttpOnly metadata.

The HTTP cache is fresh-body reuse only: no disk persistence, validator-based
revalidation, or heuristic freshness. It bypasses credentialed/cookie requests
and skips private or explicitly noncacheable responses. It is not a complete
RFC cache, and cache counters do not report all requests.

`file:` URLs access files as the process user. Loading a document and evaluating
its scripts is not an OS sandbox, a full browser origin-isolation boundary, or
an execution deadline. The remote viewer is loopback-bound but supplies no
additional authentication or session isolation mechanism beyond fresh pages.

## HTML, CSS, and layout

The tokenizer and tree builder implement insertion modes, fragments, adoption
agency behavior, foster parenting, and common error recovery. Script execution
happens after parsing with initial layout available; parser-blocking execution,
streaming script interaction, and the complete modern HTML platform are not
implemented.

CSS has a general token/component/rule parser with unknown-syntax preservation.
The [CSS guide](css.md) distinguishes that API from the renderer, which includes
selectors, cascade/inheritance, custom properties, length math, media/supports, common box properties,
rounded corners, and 2D transforms. Layout supports block and inline formatting,
floats, flex, tables, list markers, replaced elements, and positioning. Grid is
implemented with the track/placement limits in the [rendering guide](rendering.md).
Transform and border behavior remain approximations in some cases, including
mixed-style rounded borders and full stacking-context behavior.

Dirty updates rebuild the complete style/layout snapshot. Changed stylesheet
sources and newly referenced CSS backgrounds are discovered during refresh.
Dynamic `<img>` discovery remains separate. See [CSS rendering](rendering.md)
for the implemented grid, generated-content, cascade, paint, and timeline
behavior, budgets, and browser-conformance limits.

## JavaScript and events

The interpreter implements an ES subset including closures, classes, arrows,
promises, timers, regular expressions, collections, errors, and DOM/window
bindings. It is not a complete ES2024 engine or web API implementation. Accepting
`type="module"` does not implement a module graph or full module semantics.

Navigation setters queue work; `fetch` is synchronous through the runtime hook.
Timer and microtask guards do not interrupt a nonterminating script. A `Page`
reused for loading keeps its JavaScript realm and client, so it does not promise
fresh globals, timers, or console state for each document.

Click handling dispatches mouse events and then performs its anchor fallback.
The current fallback is not conditioned on browser-equivalent default-prevention
semantics. Fragment activation updates `:target` without a scrolling model and
can leave `Page::url` unchanged. The queued-navigation path also has simplified
fragment-target handling; do not assume full cross-document fragment semantics.
`set_input` changes the value attribute and fires input/change events; it is not
an implementation of full native form-control editing behavior.

The stored page title is read before scripts execute. Geometry queries consult
the most recently built layout snapshot; they do not force a synchronous style
and layout flush for every preceding script mutation.

## Text, images, and graphics

Text supports validated TTF/OpenType/CFF/TTC fonts, shaping, horizontal
bidirectional runs, and grapheme-aware fallback. A bundled DejaVu Sans supplies a
portable default. System font availability can still affect family selection and
pixels. Bidirectional formatting across nested inline boundaries is approximate.
Vertical writing, downloadable web fonts, and color emoji are not implemented.

Images support PNG, GIF, and BMP decoding and PNG encoding. JPEG and WebP are not
implemented. Recognizing a MIME type or file suffix does not imply decoder support.
The paint model uses straight RGBA8, antialiased paths and glyphs, bilinear image
scaling, rounded clipping, and 2D transforms. It is a software renderer rather than
a GPU browser compositor.

## Diagnose an unexpected result

| Observation | Likely boundary to inspect |
|---|---|
| Load succeeds but CSS or images are missing | Page console, HTTP status, format support, stylesheet/image budgets |
| `check` runs code or takes longer than parsing | It uses the complete page load path, including scripts and layout |
| Script mutation appears in DOM but geometry is old | Call `settle` after the evaluation boundary; inspect dirty-state handling |
| A newly inserted image has no pixels | Dynamic resource discovery is not part of `settle` |
| Redirected relative resources use an unexpected location | Final response URL and `Page::resolve`; `<base href>` is not consulted |
| Cached response is not reused | Credentials/cookies, cache-control, explicit freshness, size, expiry, or request-key differences |
| A batch resource fails near its memory budget | Unique bodies and duplicate clones share retention accounting |
| A click differs from the preceding remote screenshot | The click route reloads into a new page and reruns scripts |
| PNG differs on another machine | Font selection, viewport, layout inputs, and deterministic fixture configuration |
| Long-running JavaScript prevents a command returning | There is no general script execution-time limit |

Use the smallest failing fixture and the relevant focused test described in the
[development guide](development.md). Preserve the console and original inputs
when a load only partially succeeds; the process exit code alone omits that
information.
