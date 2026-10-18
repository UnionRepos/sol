//! Keeping process startup separate from command and server policy lets both
//! frontends share the kernel without making it depend on an executable.
//!
//! A rasterized remote view makes the same rendering path usable on machines
//! without a desktop, including through a forwarded loopback port over SSH.

mod cli;
mod input;
mod surf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(cli::run(&args));
}
