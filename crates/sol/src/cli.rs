//! Command dispatch and noninteractive page commands.
use crate::input::to_url;
use sol_kernel::Page;
use std::time::Instant;

pub(crate) fn run(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("shot") => shot(&args[1..]),
        Some("surf") => crate::surf::run(args.get(1).map(|s| s.as_str()).unwrap_or("3000")),
        Some("dump") => dump(&args[1..]),
        Some("check") => check(&args[1..]),
        _ => usage(),
    }
}

pub(crate) fn usage() -> i32 {
    eprintln!("usage: sol <shot|surf|dump|check> ...");
    eprintln!("  shot <url|file> <out.png> [WxH]   render to png (default 1280x900)");
    eprintln!("  surf [port]                       remote window server (default 3000)");
    eprintln!("  dump <url|file>                   outer html after load+scripts");
    eprintln!("  check <url|file>                  fetch+parse stats");
    2
}

fn shot(a: &[String]) -> i32 {
    let Some(target) = a.first() else {
        return usage();
    };
    let out = a.get(1).cloned().unwrap_or_else(|| "out.png".into());
    let (w, h) = a
        .get(2)
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| w.parse().ok().zip(h.parse().ok()))
        .unwrap_or((1280, 900));
    // Reject impossible viewports before fetching or executing the page. A bad
    // output size should not trigger expensive work or page side effects first.
    if let Err(error) = sol_paint::Canvas::validate_dimensions(w, h) {
        eprintln!("{error}");
        return 2;
    }
    let u = match to_url(target) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("url: {e}");
            return 2;
        }
    };
    let t = Instant::now();
    let mut page = Page::new(w, h);
    if let Err(e) = page.open_url(&u) {
        eprintln!("load {u}: {e}");
        return 1;
    }
    let png = page.render_png();
    if let Err(e) = std::fs::write(&out, &png) {
        eprintln!("write {out}: {e}");
        return 1;
    }
    for l in page.console_lines() {
        eprintln!("console: {l}");
    }
    eprintln!(
        "{out}: {}x{}, {} bytes, {:?} — {}",
        w,
        h,
        png.len(),
        t.elapsed(),
        page.title
    );
    0
}

fn dump(a: &[String]) -> i32 {
    let Some(target) = a.first() else {
        return usage();
    };
    let u = match to_url(target) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("url: {e}");
            return 2;
        }
    };
    let mut page = Page::new(1280, 900);
    if let Err(e) = page.open_url(&u) {
        eprintln!("load {u}: {e}");
        return 1;
    }
    println!("{}", page.outer_html());
    for l in page.console_lines() {
        eprintln!("console: {l}");
    }
    0
}

fn check(a: &[String]) -> i32 {
    let Some(target) = a.first() else {
        return usage();
    };
    let u = match to_url(target) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("url: {e}");
            return 2;
        }
    };
    let t = Instant::now();
    let mut page = Page::new(1280, 900);
    match page.open_url(&u) {
        Ok(()) => {
            let d = page.doc.borrow();
            let mut elems = 0;
            let mut texts = 0;
            for n in d.descendants(d.root) {
                match &d.node(n).data {
                    sol_dom::NodeData::Element(_) => elems += 1,
                    sol_dom::NodeData::Text(_) => texts += 1,
                    _ => {}
                }
            }
            println!(
                "{u}: ok — {elems} elements, {texts} text nodes, title {:?}, {:?}",
                page.title,
                t.elapsed()
            );
            for l in page.console_lines() {
                println!("console: {l}");
            }
            0
        }
        Err(e) => {
            eprintln!("{u}: {e}");
            1
        }
    }
}
