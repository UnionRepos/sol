//! Resolve CLI targets shared by commands and the remote window.
use sol_url::Url;
use std::path::Path;

/// Commands and remote-window routes must interpret the same target identically;
/// otherwise a file opened from the CLI could fail when revisited after a click.
pub(crate) fn to_url(s: &str) -> Result<Url, String> {
    if s.contains("://") || s.starts_with("data:") || s.starts_with("about:") {
        return Url::parse(s);
    }
    let p = Path::new(s);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(p)
    };
    Url::parse(&format!("file://{}", abs.display()))
}
