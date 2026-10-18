//! HTML views and escaping for the remote window.
use sol_url::Url;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(super) fn launcher() -> String {
    r#"<!doctype html><meta charset=utf-8><title>sol surf</title>
<style>body{font:16px system-ui;margin:3em auto;max-width:34em}
input{width:100%;padding:.5em;font:inherit}</style>
<h1>sol surf</h1>
<form action=/view><input name=u placeholder="https://example.com/" autofocus></form>
<p>Type an address. Pages are rendered server-side by sol.</p>"#
        .to_string()
}

pub(super) fn viewer(u: &str) -> String {
    // Visible attributes and nested request URLs have different delimiters.
    // HTML escaping alone cannot keep an ampersand inside a query value, and
    // URL encoding alone cannot protect the surrounding HTML attribute.
    let ue = esc(u);
    let uq = Url::encode_component(u);
    format!(
        r#"<!doctype html><meta charset=utf-8><title>sol — {ue}</title>
<style>body{{margin:0;background:#222;font:14px system-ui}}
.bar{{background:#333;padding:8px;display:flex;gap:8px}}
.bar input{{flex:1;padding:4px 8px}}</style>
<div class=bar><form action=/view style="display:flex;flex:1;gap:8px">
<input name=u value="{ue}"></form></div>
<a href="/click?u={uq}"><img src="/go?u={uq}" ismap style="display:block;max-width:100%"></a>
<p style="color:#888;padding:8px">rendered by sol kernel — <a href="/" style="color:#8af">home</a></p>"#
    )
}

pub(super) fn error_page(u: &str, e: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>error</title>
         <h1>could not load {}</h1><pre>{}</pre><p><a href=/>back</a>",
        esc(u),
        esc(e)
    )
}
