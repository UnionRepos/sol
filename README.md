# sol — a self-contained browser kernel in Rust

[![CI](https://github.com/UnionRepos/sol/actions/workflows/ci.yml/badge.svg)](https://github.com/UnionRepos/sol/actions/workflows/ci.yml)

`sol` is a browser kernel with in-tree HTML, CSS, layout, JavaScript, DOM,
and image codecs. Networking and font rendering use maintained Rust libraries:
`ureq`/rustls for HTTP and HTTPS, `flate2` for HTTP deflate, `owned_ttf_parser`
for validated font access, `rustybuzz` and Unicode text libraries for shaping
and bidirectional text, and `tiny-skia` for antialiased paths and glyph rasterization.

```
fetch → HTML parse → CSS cascade → layout → display list → raster → PNG
                ↘ JS eval / DOM mutation ↗   (refresh after invalidation)
```

## Documentation

| Guide | Read it for |
|---|---|
| [Building, commands, and embedding](docs/usage.md) | First build, exact command behavior, exit codes, remote viewing, runnable Rust examples |
| [Architecture and design rationale](docs/architecture.md) | Ownership, page lifecycle, execution order, module boundaries, caching, rendering geometry |
| [CSS parsing and evaluation](docs/css.md) | Syntax APIs, preservation, selectors, variables, math, conditional rules, and rendering boundaries |
| [Continuous integration and delivery](docs/ci-cd.md) | Validation gates, release archives, manual runs, version-tag publication |
| [Development and maintenance](docs/development.md) | Source navigation, focused tests, fixtures, extension invariants, comment conventions |
| [Supported behavior and budgets](docs/limits.md) | Allocation limits, implemented subsets, state-lifetime constraints, diagnosis |

## Crate map (bottom → top)

| crate | what it implements |
|---|---|
| `sol_util` | generational arenas, string interning, timers |
| `sol_url` | WHATWG-style URL parse/join/serialize, origins, percent-encoding |
| `sol_net` | ureq HTTP/HTTPS client: pooled connections, proxies, bounded gzip/Brotli/deflate, redirects, cookies, `file:`/`data:`; shared bounded in-tree inflater for image codecs; parallel GET batches and freshness cache |
| `sol_tls` | standalone experimental TLS 1.3 implementation and crypto test vectors; browser networking uses rustls |
| `sol_img` | bounded PNG (+interlace), GIF, BMP decoders; PNG encoder and fallible image allocation |
| `sol_dom` | arena DOM with generational `NodeId`, dirty-flag subtrees, serialization |
| `sol_html` | HTML5 tokenizer + tree builder (insertion modes, adoption agency, foster parenting, fragments) |
| `sol_css` | General CSS token/component AST, selector engine, cascade/inheritance, custom properties, length math, media/supports conditions, computed `Style`, UA stylesheet |
| `sol_text` | validated TTF/OpenType/CFF/TTC access, tiny-skia glyph rasterizer, script shaping, bidirectional runs, grapheme-aware fallback, cached font discovery, bundled DejaVu Sans |
| `sol_layout` | box-tree builder; block, inline/IFC, floats, flexbox, table, abs/fixed positioning, list markers |
| `sol_paint` | display list + software canvas: clips, antialiased rectangles/text, correct straight-alpha blending, bilinear image scaling, rounded borders/clips, 2D transforms |
| `sol_js` | ES-subset interpreter: parser, tree-walk eval, mark-sweep GC, regex engine, microtasks, timers, DOM/window bindings, `fetch` |
| `sol_kernel` | `Page` orchestrator: navigation, subresources, script order, incremental re-style/layout, hit-test, click/anchor navigation, pseudo-state (`:hover`/`:active`/`:focus`/`:visited`/`:target`) |
| `sol` (bin) | command line + remote-window server |

### Internal module boundaries

The crate roots retain the public API and ownership of shared state. Private
modules group behavior by responsibility, so callers continue to use `Page`,
`Client`, `Response`, and `CookieJar` through their existing crate paths.

| crate | modules and responsibilities |
|---|---|
| `sol_css` | `syntax`: tokens and general AST; `parser`: rendering adapter; `selector`/`matching`: selector grammar and DOM evaluation; `conditions`: media/supports; `variables`: substitution; `math`: dimensional lengths; `style`: cascade |
| `sol_kernel` | `navigation`: document loading and script order; `resources`: ordered stylesheets and deduplicated images; `runtime`: JS host hooks and event turns; `rendering`: style/layout/paint and transformed hit testing; `interaction`: pseudo-state and input dispatch |
| `sol_net` | crate root: client configuration, request/cache policy and redirects; `transport`: HTTP/TLS and decompression; `local`: file/data URLs; `response`: metadata; `encoding`: text/base64; `cookies`: scope and expiry; `body`: shared byte limits; `resources`: caching and parallel batches |
| `sol` | `cli`: command dispatch and page commands; `input`: shared URL/path resolution; `surf`: listener and routes; `surf::http`: request framing and responses; `surf::views`: escaped HTML |

Keep new behavior in the module that owns its policy. Rendering geometry and
hit testing share transform handling; local and HTTP responses share body-limit
helpers; commands and server routes share target resolution. Keep module internals
private and re-export supported types from crate roots when extending the API.

## CLI

```bash
cargo build -p sol

./target/debug/sol shot demo/page.html /tmp/sol-demo.png 1000x640
```

The built executable is `target/debug/sol` (or `target/release/sol` after a release
build). Its commands are:

```text
sol shot <url|file> [out.png] [WxH]   # render a viewport to PNG
sol dump <url|file>                  # serialized DOM after scripts run
sol check <url|file>                 # page-load stats, including scripts/layout
sol surf [port]                      # remote window: http://127.0.0.1:3000/
```

`surf` is a remote window for headless machines: pages are rasterized
server-side and shipped as PNG; `<img ismap>` click-through follows links.
Over SSH: `ssh -L 3000:127.0.0.1:3000 host` then open the URL locally.
Each image and click request loads a fresh page; the remote window does not retain
a browsing session between requests.

Try it: `sol shot demo/page.html out.png 1000x640`

## Design notes

- **Focused dependencies.** External crates serve networking, fonts, and graphics
  rendering. HTML5 tree construction, CSS, flex/table layout, JavaScript with GC,
  DOM, URL parsing, and image codecs remain in-tree. `sol_tls` remains available
  separately and is not used as a fallback for failed HTTPS connections.
- **Bounded network I/O.** `Client::default()` and `Client::new()` both verify
  HTTPS certificates and hostnames using system roots. Environment proxies and
  `NO_PROXY` are honored; `Client::set_proxy` overrides them per client. Exchanges
  default to a 25-second deadline and a 64 MiB decoded-body limit; `timeout` and
  `max_body_bytes` are configurable. Compressed wire bodies are also bounded.
  Redirects retain POST bodies for 307/308, convert POST to GET for 301/302,
  remove credentials across origins, and reject HTTPS downgrades and local-file
  targets. Cookie matching respects host-only scope and path boundaries.
- **Parallel resources and caching.** `Client::get_many` deduplicates URLs,
  returns results in input order, uses at most eight workers, and retains at most
  64 MiB of response bodies per batch. `Page::resource_parallelism` defaults to
  four. Stylesheets retain document cascade order; blocking scripts still fetch
  and execute sequentially. Duplicate images share decoded pixels, with a 64 MiB
  page budget. The shared LRU HTTP cache holds at most 128 entries and 32 MiB of
  bodies, uses explicit `max-age`/`Expires` freshness with `Date`/`Age`, and caps
  freshness at one hour. It skips credentialed requests, cookies, `no-store`,
  `no-cache`, private responses, and `Vary: *`; unsafe methods invalidate cached
  versions of their target. `Client::clear_cache` and `cache_stats` expose cache
  control and diagnostics; `Page::network_client` exposes per-page settings.
- **Bounded image decoding.** `Image::try_new` and all decoders enforce at most
  16384 pixels per side, 16 megapixels, and 64 MiB of encoded input. PNG validates
  chunk lengths/CRCs, header combinations, filters, palettes, Adler checksums,
  and exact scanline sizes. Its in-tree DEFLATE output is bounded before growth.
  GIF validates frame bounds and sub-blocks, limits LZW output to the frame and
  dictionary size to 4096, and requires an end code. BMP validates headers,
  palettes, strides, color masks, offsets, and complete pixel rows.
- **Portable fonts and rendering.** TTF, OpenType/CFF, and TTC discovery skips
  malformed files and symlink cycles, with a 64 MiB font-data budget. A bundled
  [DejaVu Sans font](crates/text/assets/LICENSE-DejaVu.txt) supplies readable text
  when no system fonts are available. Font sizes are bounded to 512 px and glyph
  bitmaps to 2048 pixels per side. Canvas allocation has a fallible `try_new` API,
  a 16-megapixel budget, and a 16384-pixel limit per side; the CLI validates these
  limits before loading a page. `Face::new` constructs a face; `Face::shape`
  returns glyph IDs, source clusters, per-glyph fonts, positions, and mark offsets.
  Measurement and painting use this same shaping path. Line layout reorders
  horizontal bidirectional runs, including mixed numbers and Latin/RTL text.
- **Rounded and transformed painting.** Radius shorthands support elliptical
  corners and longhand overrides. Rounded backgrounds, borders, and overflow
  clips use antialiased paths. Translate, scale, rotate, skew, and matrix
  transforms compose around `transform-origin` and affect descendants, images,
  text, hit testing, and DOM bounding rectangles. Clip masks retain at most
  64 MiB; rendering clips further content when this budget is exhausted.
- **Invalidation-driven pipeline.** The DOM carries dirty flags; `Page::settle`
  avoids style/layout work on clean turns. A dirty refresh currently rebuilds
  the full document rather than only the affected subtree. Resource discovery
  runs during loading and is not repeated by settling.
- **Event-turn model.** `location` changes are queued so document replacement
  happens after callbacks return. Microtasks drain before timer pumping.
  JavaScript evaluation and its fetch hook are synchronous; timer-round limits
  do not impose an execution deadline on scripts.
- **Correct-by-spec details.** body→canvas background propagation,
  anonymous flex items per CSS-FLEXBOX §4, whitespace-only flex item
  dropping, `<script>`/`<style>` raw-text extraction.

## Testing

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
```

Crypto primitives are verified against RFC test vectors (SHA-2, HKDF,
ChaCha20-Poly1305, AES-GCM, X25519). The kernel suite renders real pages
end-to-end (`data:`/`file:` URLs — no network needed).

Integration tests use local HTTP/HTTPS servers for certificate rejection,
connection reuse, informational responses, redirects, cookies, compression,
truncation, and deadlines. Font tests use bundled or synthetic fixtures, covering
malformed files, CFF cubic outlines, collections, and machines without system
fonts. Paint tests cover transparent compositing, fractional edges, image
interpolation, allocation limits, nested transforms, and rounded clipping. Image
regressions exercise truncation, corrupt headers, output budgets, and malformed
DEFLATE streams. Text tests cover ligatures, Arabic joining, combining marks,
bidirectional numbers, and fallback metrics. Resource tests check concurrency,
deduplication, freshness, eviction, credentials, and CSS/script order.
Executable tests cover path resolution, script-visible DOM output, PNG files,
exit codes, HTTP routes, escaping, and click coordinate forms.
No external services are required by tests.

The screenshot fixture in `crates/paint/tests/fixtures/rounded-transform.html`
has a checked-in PNG baseline. Review intentional changes visually, then update:

```bash
SOL_UPDATE_SCREENSHOTS=1 cargo test -p sol_paint --test paint rounded_transform_screenshot_matches_fixture --offline
```

CI runs the suite in both debug and release profiles. Version tags build and
publish tested Linux archives; see [CI/CD](docs/ci-cd.md) for the release process.

## Known limits

Block/inline/flex/table + positioning cover the common box model; no CSS
grid yet. JS implements a working ES subset (closures, classes, arrows,
promises, regex, Map/Set, errors) — not full ES2024. Text is horizontal; no vertical
writing, downloadable web fonts, or color emoji. Bidirectional formatting across
nested inline boundaries is still approximate. Images support PNG/GIF/BMP;
JPEG/WebP are not implemented. Transforms are 2D; mixed-style rounded borders
are approximated. One cookie jar with no public-suffix-list validation. The cache
is memory-only and does not revalidate stale entries with validators. Each worker
uses synchronous HTTP/1.1 (no HTTP/2 or HTTP/3).

## License

Project code is licensed under the [Apache License 2.0](LICENSE).
The bundled DejaVu Sans font retains its [separate license](crates/text/assets/LICENSE-DejaVu.txt).
External dependencies retain their respective licenses.
