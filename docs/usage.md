# Building, commands, and embedding

[Project overview](../README.md) · [Architecture](architecture.md) ·
[Development](development.md) · [Limits and budgets](limits.md)

## Requirements and first build

Run commands from the repository root. You need Cargo and a Rust toolchain capable
of building the versions in `Cargo.lock`; the workspace uses the Rust 2021 edition
and does not declare a minimum supported Rust version. The initial build needs
access to the locked dependencies unless they are already cached.

No graphical desktop or external browser is required. The text crate includes
DejaVu Sans as a fallback, so a system font installation is optional. HTTPS uses
platform certificate verification. Proxy settings are inherited by the network
client unless an embedder overrides them.

```bash
cargo build --locked -p sol
./target/debug/sol shot demo/page.html /tmp/sol-demo.png 1000x640
./target/debug/sol dump demo/page.html
./target/debug/sol check demo/page.html
```

For an optimized executable:

```bash
cargo build --locked --release -p sol
./target/release/sol shot demo/page.html /tmp/sol-demo-release.png 1000x640
```

`cargo run --locked -p sol -- <command> ...` is equivalent to invoking the debug
binary after building it. Add `--offline` to Cargo commands once dependencies are
cached to prevent dependency downloads. Offline builds fail if required packages
are missing; the repository does not vendor its external dependencies.

## Target interpretation

Commands and remote-window routes use the same target resolver:

| Input | Interpretation |
|---|---|
| Contains `://` | Parsed as a URL |
| Starts with `data:` or `about:` | Parsed as a URL |
| Anything else | Filesystem path, relative to the process working directory unless absolute |

A parsed scheme is not necessarily loadable. The network client accepts `http`,
`https`, `file`, and `data`; for example, an `about:` target currently reaches an
unsupported-scheme error. Scheme recognition here is case-sensitive. Bare paths
are joined to the current directory rather than canonicalized.

Quote targets containing shell metacharacters. For portable binary fixtures,
prefer explicit URLs with percent encoding or base64 data URLs. The bare-path
adapter interpolates a path into a file URL and does not fully escape characters
such as `#` and `?` in filenames.

```bash
./target/debug/sol dump 'data:text/html,<title>Example</title><p>Hello</p>'
./target/debug/sol shot 'file:///tmp/page.html' /tmp/page.png 800x600
```

Relative page resources resolve against the page's URL, including the final URL
after redirects. The kernel currently resolves through `Page::url`; a parsed
`Document::base_href` is not consulted by that resolver.

## Screenshot command

```text
sol shot <target> [output.png] [WxH]
```

The output defaults to `out.png` in the working directory. The viewport defaults
to 1280×900. Dimensions are positional: to specify them, supply the output path
first. The separator is lowercase `x`.

If the size does not parse, the command uses the default viewport. A parsed size
outside the canvas limits fails before loading the page. Each side must be
1–16384 pixels, with at most 16 × 1024 × 1024 pixels in total.

The command loads the page, runs its scripts and bounded timer rounds, renders
the current viewport, and writes PNG bytes. It does not wait for arbitrary future
timers or capture a full scrollable document beyond the viewport. Console lines,
output dimensions, byte count, elapsed time, and the stored page title go to stderr.

```bash
./target/debug/sol shot demo/page.html /tmp/preview.png 800x600
```

## DOM dump command

```text
sol dump <target>
```

Serialized DOM after loading and script execution goes to stdout. Console output
goes to stderr, so stdout can be redirected as markup:

```bash
./target/debug/sol dump demo/page.html > /tmp/sol-dom.html
```

This is a serialized tree, not the original input source. Parser repair and script
mutations can change it. It is also not a screenshot or a dump of computed styles.

## Check command

```text
sol check <target>
```

A successful check prints the URL, element and text-node counts, stored title,
and elapsed time to stdout. Console messages also go to stdout for this command.
Loading errors go to stderr.

Despite its short name, `check` creates a `Page` and uses the normal load pipeline,
including resources, initial style/layout, scripts, microtasks, and bounded timer
rounds. It is not a parse-only syntax checker and does not suppress script side
effects. For syntax-only JavaScript checking, use `sol_js::Js::parse` in an embedder.

## Remote window

```bash
./target/debug/sol surf 3000
```

The default port is 3000. The listener binds to `127.0.0.1`; open
`http://127.0.0.1:3000/` in a browser. The launcher accepts an address and displays
a PNG produced by the kernel. Image-map clicks submit coordinates for hit testing
and link activation. Rendering uses a fixed 1280×900 viewport.

For a remote machine, forward the loopback listener through SSH:

```bash
ssh -L 3000:127.0.0.1:3000 host
```

Run `sol surf 3000` on the remote machine, then open the local forwarded address.
The server process's working directory determines how bare paths are interpreted.

| Route | Result |
|---|---|
| `/` | Launcher HTML |
| `/view?u=<encoded-target>` | HTML wrapper with the address field and image |
| `/go?u=<encoded-target>` | Rendered PNG, or an HTML error page |
| `/shot?u=<encoded-target>` | Alias for `/go` |
| `/click?u=<encoded-target>&x=10&y=20` | Page load, click dispatch, redirect to `/view` |
| `/click?u=<encoded-target>?10,20` | Equivalent image-map coordinate form |

Encode the entire target as a query value. A nested URL's `&` belongs to the target,
not the server route's parameter list. Viewer HTML escaping and query encoding
serve different purposes and are both required.

Every image and click request loads a new page. Cookies, DOM mutations, globals,
and timers from the preceding request are not a session. A click re-evaluates the
source before dispatch, so pages whose content changes between loads may not match
the preceding screenshot. Connections are handled sequentially. There is no
keyboard, scrolling, or continuous browser-event forwarding protocol.

The server is a small local inspection frontend with no authentication, request
size limits, or socket deadline configuration. Its default empty image target is
`about:blank`, which currently produces an unsupported-scheme error. Supply an
explicit file, HTTP(S), or data target.

## Exit codes and diagnostic output

| Exit code | Meaning |
|---|---|
| `0` | The command completed its main operation |
| `1` | Loading, output writing, or server binding failed |
| `2` | Usage error, invalid target syntax, invalid port, or rejected viewport |

Subresource and script errors can be logged while the command still returns zero.
Inspect console output as well as the exit status when diagnosing a partial page.
The page title is captured before scripts run and is not automatically refreshed
from subsequent title mutations.

## Embed a page

Within this workspace, another crate can declare path dependencies on `sol_kernel`,
`sol_url`, and `sol_paint`. For a crate outside the repository, adjust the paths to
point at this checkout. The crates are not installed by building the executable.

```toml
[dependencies]
sol_kernel = { path = "../sol/crates/kernel" }
sol_url = { path = "../sol/crates/url" }
sol_paint = { path = "../sol/crates/paint" }
```

This complete example uses no external service:

```rust
use sol_kernel::Page;
use sol_paint::Canvas;
use sol_url::Url;

fn main() -> Result<(), String> {
    Canvas::validate_dimensions(320, 200)?;
    let mut page = Page::new(320, 200);
    let base = Url::parse("file:///tmp/sol-example.html")?;
    page.load_html(
        "<title>Embedded</title><body><div id='message'>before</div></body>",
        &base,
    )?;
    page.js
        .eval("document.getElementById('message').textContent = 'after'")
        .map_err(|error| format!("{error:?}"))?;
    page.settle();
    assert!(page.outer_html().contains(">after</div>"));
    let png = page.render_png();
    assert!(png.starts_with(b"\x89PNG"));
    Ok(())
}
```

`Page::new` does not return a dimension-validation error. Validate before loading
when dimensions come from a caller. `render` uses the infallible canvas constructor;
lower-level consumers can use `Canvas::try_new` when allocation must be fallible.

`load_html` needs a base URL even when the source is already in memory. It supplies
relative-resource resolution and the runtime's document location context.
`open` accepts a URL string; it does not apply the executable's bare-path adapter.
For a reused page, `load_html` retains the existing realm and network client. Make
a new `Page` if independent JavaScript globals and runtime queues are required.

Keep document borrows short. Drop a `page.doc.borrow()` or `borrow_mut()` guard
before calling JavaScript, page loading, settling, or input dispatch, since those
operations can access the same document through runtime callbacks.

For a viewport change, validate dimensions, set `page.js.rt.viewport`, then call
`page.restyle()` and `page.relayout()`. Changing the public tuple alone does not
mark the page dirty. The same consideration applies to direct changes to
`page.interact`: explicitly refresh styles/layout before depending on the result.
`Page::set_input` and `Page::click` include their own settling behavior, with the
input/navigation limitations described in [the architecture guide](architecture.md).

## Configure networking and inspect cache behavior

A page exposes its client through `network_client()`. Scope the returned borrow
before loading or evaluating code that may fetch. For a standalone client, depend
on `sol_net` and `sol_url`:

```rust
use sol_net::Client;
use sol_url::Url;
use std::time::Duration;

fn main() -> Result<(), String> {
    let mut client = Client::new();
    client.set_proxy(None);
    client.timeout = Duration::from_secs(5);
    client.max_body_bytes = 1024 * 1024;
    let url = Url::parse("data:text/plain;charset=utf-8,hello")?;
    let response = client.get(&url)?;
    assert_eq!(response.status, 200);
    assert_eq!(response.text(), "hello");
    assert_eq!(client.cache_stats().entries, 0);
    Ok(())
}
```

TLS verification is enabled by default. `set_proxy(None)` overrides inherited
proxies for that client; omit it to keep environment proxy behavior. The timeout
applies to each HTTP exchange, including its body, rather than one deadline for an
entire redirect chain. `max_body_bytes` controls decoded responses; other wire,
batch, image, and page budgets remain separate.

For batches, call `get_many(&urls, parallelism)`. Results match input positions and
can fail individually. Concurrency is clamped to 1–8. Requests depending on a prior
response's cookies must be sequential. The cache is in memory and shared across
client clones; `clear_cache()` also resets counters. Local/data URLs do not enter
the HTTP freshness cache.

The JavaScript `document.cookie` binding currently uses a separate runtime string
and does not read or modify `Client::cookies`. Use the network cookie jar directly
when configuring request cookies from an embedder.
