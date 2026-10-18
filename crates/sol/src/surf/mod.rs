//! Remote-window listener and routes.
//!
//! GET / launches the viewer; /view wraps a /go (or /shot) PNG;
//! /click dispatches input and redirects to the resulting page URL.
mod http;
mod views;

#[cfg(test)]
mod tests;

use self::http::{read_target, respond, respond_html, respond_not_found, respond_redirect};
use self::views::{error_page, launcher, viewer};
use crate::input::to_url;
use sol_kernel::Page;
use sol_url::Url;
use std::io::BufReader;
use std::net::TcpListener;

pub(crate) fn run(port: &str) -> i32 {
    let port: u16 = match port.parse() {
        Ok(p) => p,
        Err(_) => return crate::cli::usage(),
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind 127.0.0.1:{port}: {e}");
            return 1;
        }
    };
    eprintln!("sol surf on http://127.0.0.1:{port}/ (Ctrl-C to stop)");
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        handle(stream);
    }
    0
}

fn handle(stream: std::net::TcpStream) {
    let mut r = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let Some(target) = read_target(&mut r) else {
        return;
    };
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    fn qget(query: &str, k: &str) -> String {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(a, _)| *a == k)
            .map(|(_, b)| sol_url::pct_decode(&b.replace('+', "%20")))
            .unwrap_or_default()
    }
    let q = |k: &str| qget(query, k);

    match path {
        "/" => {
            let body = launcher();
            respond_html(stream, &body);
        }
        "/view" => {
            let u = q("u");
            let body = viewer(&u);
            respond_html(stream, &body);
        }
        "/go" | "/shot" => {
            let u = q("u");
            let u = if u.is_empty() {
                "about:blank".into()
            } else {
                u
            };
            match render_url(&u) {
                Ok(png) => respond(stream, "image/png", &png),
                Err(e) => respond_html(stream, &error_page(&u, &e)),
            }
        }
        "/click" => {
            // <img ismap> appends "?x,y" to the href, producing a second '?'
            // in the query string. Take coords from the tail; fall back to
            // explicit x=/y= params.
            let (uq, xy) = match query.rsplit_once('?') {
                Some((a, b)) if b.contains(',') => (a, b),
                _ => (query, ""),
            };
            let u = qget(uq, "u");
            let (mut x, mut y) = (0.0f32, 0.0f32);
            if let Some((xs, ys)) = xy.split_once(',') {
                x = xs.parse().unwrap_or(0.0);
                y = ys.parse().unwrap_or(0.0);
            }
            if xy.is_empty() {
                x = qget(uq, "x").parse().unwrap_or(0.0);
                y = qget(uq, "y").parse().unwrap_or(0.0);
            }
            match click_url(&u, x, y) {
                Ok(next) => {
                    let loc = format!("/view?u={}", Url::encode_component(&next));
                    respond_redirect(stream, &loc);
                }
                Err(e) => respond_html(stream, &error_page(&u, &e)),
            }
        }
        _ => respond_not_found(stream),
    }
}

fn render_url(u: &str) -> Result<Vec<u8>, String> {
    let url = to_url(u)?;
    // Each request is independent because this frontend has no page-session
    // identifier. Reusing one global Page would let one visitor's navigation
    // replace another visitor's document between their image and click requests.
    let mut page = Page::new(1280, 900);
    page.open_url(&url)?;
    Ok(page.render_png())
}

/// Click through: open page, synthesize click, report resulting URL.
fn click_url(u: &str, x: f32, y: f32) -> Result<String, String> {
    let url = to_url(u)?;
    let mut page = Page::new(1280, 900);
    page.open_url(&url)?;
    page.click(x, y);
    Ok(page
        .url
        .map(|u| u.serialize())
        .unwrap_or_else(|| u.to_string()))
}
