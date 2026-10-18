# Development and maintenance

[Project overview](../README.md) · [Architecture](architecture.md) ·
[Usage and embedding](usage.md) · [Limits and budgets](limits.md)

## Find the right boundary

Start from a failing behavior and identify the earliest stage that diverges.
A wrong screenshot can originate in parsing, cascade, font selection, layout,
display-list construction, or compositing. Changing a downstream stage to conceal
an upstream error can make other pages fail in less obvious ways.

| Symptom or feature | Start here | Existing evidence to inspect |
|---|---|---|
| URL joins, origins, request targets | [`sol_url`](../crates/url/src/lib.rs) | URL unit tests |
| Redirects, headers, TLS, compression, deadlines | [`sol_net`](../crates/net/src/lib.rs), [`transport`](../crates/net/src/transport.rs) | [`network.rs`](../crates/net/tests/network.rs) |
| Cache identity, freshness, batch ordering | [`net::resources`](../crates/net/src/resources.rs) | Resource unit tests and network integration tests |
| Corrupt images or allocation limits | [`sol_img`](../crates/img/src/lib.rs) | [`img.rs`](../crates/img/tests/img.rs), [`stability.rs`](../crates/img/tests/stability.rs) |
| Node lifetime, ID lookup, dirty state | [`sol_dom`](../crates/dom/src/lib.rs), [`sol_util`](../crates/util/src/lib.rs) | Inline DOM/arena tests |
| Repaired HTML structure or fragment context | [`html::builder`](../crates/html/src/builder.rs), [`tokenizer`](../crates/html/src/tokenizer.rs) | [`parse.rs`](../crates/html/tests/parse.rs) |
| CSS tokenization, general rules, recovery | [`css::syntax`](../crates/css/src/syntax/mod.rs) | [`syntax.rs`](../crates/css/tests/syntax.rs), [CSS API guide](css.md) |
| Selectors, inheritance, declarations | [`css::style`](../crates/css/src/style.rs), [`selector`](../crates/css/src/selector.rs), [`parser`](../crates/css/src/parser.rs) | [`css.rs`](../crates/css/tests/css.rs), [`modern.rs`](../crates/css/tests/modern.rs) |
| Shaping, fallback, glyph output | [`sol_text`](../crates/text/src/lib.rs) | [`text.rs`](../crates/text/tests/text.rs) |
| Box geometry, line wrapping, flex/table layout | [`sol_layout`](../crates/layout/src/lib.rs) | [`layout.rs`](../crates/layout/tests/layout.rs) |
| Alpha, scaling, clips, transforms | [`sol_paint`](../crates/paint/src/lib.rs), [`graphics`](../crates/paint/src/graphics.rs) | [`paint.rs`](../crates/paint/tests/paint.rs) |
| Language syntax, evaluation, builtins, DOM bindings | [`sol_js`](../crates/js/src/lib.rs) | [`js.rs`](../crates/js/tests/js.rs) |
| Load order, state lifetime, hit testing | [`sol_kernel`](../crates/kernel/src/lib.rs) and private modules | [`kernel.rs`](../crates/kernel/tests/kernel.rs) |
| Argument behavior, output streams, server routes | [`sol`](../crates/sol/src/main.rs) | [`cli.rs`](../crates/sol/tests/cli.rs), [`surf/tests.rs`](../crates/sol/src/surf/tests.rs) |
| Experimental cryptographic primitives | [`sol_tls`](../crates/tls/src/lib.rs) | Inline RFC/vector tests |

Avoid adding a dependency on `Page` to a lower-level crate. Pass the inputs that
stage needs, or introduce a callback/trait boundary when host behavior is required.
`PseudoState` and the JavaScript runtime hooks are existing examples of that
separation. Exact crate dependencies live in their Cargo manifests.

## Automation

The [CI/CD guide](ci-cd.md) describes GitHub Actions validation, debug/release test
runs, workflow linting, release packaging, and version-tag publication. The tested
Rust toolchain is pinned in `rust-toolchain.toml`.

## Build and inspect

```bash
cargo build --locked -p sol
cargo check --workspace --offline
cargo tree --workspace --offline
cargo metadata --no-deps --format-version 1 --offline
```

The first command can download locked dependencies. The remaining commands assume
they are cached. Avoid changing dependency versions as an incidental consequence
of a source refactor. If dependencies intentionally change, review both the
manifest and lockfile and explain why the new boundary needs them.

For API documentation:

```bash
cargo doc --workspace --no-deps --offline
```

Open `target/doc/sol_kernel/index.html` for `Page` and
`target/doc/sol_net/index.html` for the client APIs. Private implementation details
can be included with `--document-private-items` when investigating internal code.

## Validation workflow

Use a focused check while implementing, then the workspace checks relevant to the
change. The standard validation commands are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo test --workspace --offline
```

Clippy warnings are treated as errors across the workspace and all targets.
Resolve new diagnostics in the relevant implementation instead of suppressing
the lint; keep the documented command clean when extending the project.

Examples of focused runs:

```bash
cargo test -p sol_net --test network --offline
cargo test -p sol_kernel --test kernel --offline
cargo test -p sol --offline
cargo test -p sol_img --test stability --offline
cargo test -p sol_text --test text --offline
```

Do not run `--offline` as a promise that a test can never open a socket: it affects
Cargo dependency resolution. Network integration tests intentionally bind local
HTTP/HTTPS listeners. They do not require an external service. Sandboxes must
allow loopback sockets for those tests.

Documentation-only edits usually need formatting, link checks, and API-doc builds.
Run any new or modified examples as well. Add regression tests when there is a
behavioral risk, rather than tests that only mirror a private function's structure.

## Test layers and fixtures

Keep a regression at the smallest layer that can express the failure. Then use
an integration test when correctness depends on cooperation between layers.

- Parser fixtures should assert tree structure or token behavior, including
  malformed input and fragment context when relevant.
- Style/layout tests should assert computed values and geometry before relying
  on pixels. That makes failures easier to localize.
- Image and inflater cases should cover truncation, malformed metadata, and
  rejected output growth as well as successful decoding.
- Network fixtures should use local listeners and explicit proxy configuration
  so environment proxy settings do not decide the result. Include deadlines for
  fixture servers instead of allowing a failed exchange to hang the suite.
- Kernel tests connect script mutation, resource ordering, geometry, and pixels.
  Preserve tests that compare transformed DOM rectangles with painted hits.
- CLI tests execute the built binary to cover exit status, stdout/stderr, path
  interpretation, and output files. Server route tests cross request framing,
  routing, views, and response writing through loopback sockets.

The [HTTPS fixture note](../crates/net/tests/fixtures/README.md) explains the
self-signed certificate and public test key. Default certificate rejection is
part of the regression suite. Keep any verification override local to its test.
The bundled font has its own [license notice](../crates/text/assets/LICENSE-DejaVu.txt).

## Screenshot baselines

The paint suite includes
[`rounded-transform.html`](../crates/paint/tests/fixtures/rounded-transform.html)
and its [PNG baseline](../crates/paint/tests/fixtures/rounded-transform.png).
If a rendering change intentionally updates the expected output, inspect the image
before accepting a regenerated baseline:

```bash
SOL_UPDATE_SCREENSHOTS=1 cargo test -p sol_paint --test paint rounded_transform_screenshot_matches_fixture --offline
```

Then run the same test without the update variable. A new baseline must reflect
an understood visual change; it must not be used to hide an unexplained failure.
For new deterministic fixtures, choose controlled fonts and small viewports, and
assert geometry or colors directly when a full screenshot adds little evidence.

## Invariants to preserve

### Shared state and mutation

Use document mutation methods when possible. Raw node edits must preserve ID
indexing, parent/child relationships, and dirty flags. A `NodeId` is generational;
retaining an ID is not proof the node is still alive, and IDs should not be treated
as portable across documents or document replacements.

End `RefCell` borrows before invoking scripts, events, loads, or settling. Runtime
callbacks can re-enter the same shared state. Changing a field to `Arc<Mutex<_>>`
alone would not make the rest of the `Rc`-based page transferable across threads.
A threading redesign needs a complete ownership and callback plan.

### Resource and execution order

Keep stylesheet cascade order independent from fetch completion order. Preserve
sequential dependencies between scripts. Batch cookie updates have deterministic
input-order replay; requests that need newly set cookies must not be placed in
the same independent batch.

Dirty flags do not schedule resource discovery. A dynamic resource feature needs
an explicit discovery, deduplication, cancellation/lifetime, and budget design;
a style refresh alone cannot supply that behavior.

### Coordinates and text

Keep painting, hit testing, clipping, and JavaScript rectangles in agreement.
Reuse transform helpers and match exclusions for anonymous/text boxes. When
changing shaping or fallback, verify that measurement and paint still use the
same face selection and glyph path. Test mixed direction, combining marks, and
transparent image edges when those paths are affected.

### Bounds and ownership

Check untrusted sizes before allocation or decompression growth. Separate
per-response limits from batch retention and decoded-image budgets. Shared image
pixels and cloned response bodies have different ownership costs; changing an
`Rc` to an owned clone can alter memory use even when pixels are unchanged.

Cache identity must continue to separate TLS policy and request representations.
Do not let a transport shortcut bypass redirect origin checks or cookie scope.
When adding an HTTP cache feature, specify how credentials, `Vary`, validators,
unsafe methods, and response age affect it before admitting more responses.

## Write comments that explain decisions

Prefer a comment when a plausible simplification would break an invariant that
is not visible from the local statement. Explain the consequence and the reason
the implementation avoids it. Keep basic mechanics in names and API/project
documentation rather than repeating them beside the code.

For example, the bounded-read comment explains why the reader takes one byte
beyond the requested limit: otherwise an exactly-full body and an oversized body
would be indistinguishable. The stylesheet comment explains why source positions
survive fetch deduplication: repeated links still have separate cascade positions.
Neither needs a line-by-line description of the loop.

Keep rationale close to the decision it protects. When behavior changes, review
its comment and the corresponding architecture/limits section together. Describe
an observed constraint rather than inventing historical intent or claiming a
standard is fully implemented because one rule is handled.

## Git identity in this workspace

Do not change Git configuration. If a commit is requested, pass the required
identity inline:

```bash
git -c user.name="lemonhub-io" -c user.email="lemonhub.io@icloud.com" commit ...
```

Use the same identity for amendments, rebases, merges, and tags. Correct an
unpushed commit made with another identity before pushing it. Source changes and
documentation work do not by themselves require creating a commit.
