//! Exercise the executable's public command and output contracts.
use std::path::PathBuf;
use std::process::{Command, Output};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sol-cli-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sol"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn invalid_commands_and_arguments_keep_exit_codes() {
    let workspace = Workspace::new();
    for args in [
        &[][..],
        &["unknown"][..],
        &["shot"][..],
        &["dump"][..],
        &["check"][..],
        &["surf", "invalid"][..],
    ] {
        let output = workspace.run(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("usage: sol"));
    }
    let output = workspace.run(&["shot", "missing.html", "out.png", "20000x60"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!workspace.0.join("out.png").exists());
    let output = workspace.run(&["dump", "missing.html"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("load file://"));
}

#[test]
fn page_commands_resolve_paths_execute_scripts_and_write_png() {
    let workspace = Workspace::new();
    std::fs::write(
        workspace.0.join("page.html"),
        "<!doctype html><title>CLI fixture</title><body><div id='message'>before</div>\
         <script>document.getElementById('message').textContent='after';console.log('ready');</script></body>",
    )
    .unwrap();

    let dump = workspace.run(&["dump", "page.html"]);
    assert!(dump.status.success(), "{:?}", dump);
    assert!(String::from_utf8_lossy(&dump.stdout).contains(">after</div>"));
    assert!(String::from_utf8_lossy(&dump.stderr).contains("console: ready"));

    let check = workspace.run(&["check", "page.html"]);
    assert!(check.status.success(), "{:?}", check);
    let stats = String::from_utf8_lossy(&check.stdout);
    assert!(stats.contains("elements,"));
    assert!(stats.contains("title \"CLI fixture\""));
    assert!(stats.contains("console: ready"));

    let shot = workspace.run(&["shot", "page.html", "page.png", "80x60"]);
    assert!(shot.status.success(), "{:?}", shot);
    let png = std::fs::read(workspace.0.join("page.png")).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 80);
    assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 60);
    assert!(String::from_utf8_lossy(&shot.stderr).contains("console: ready"));
}
