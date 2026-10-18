# Continuous integration and delivery

[Project overview](../README.md) · [Development](development.md) ·
[Usage](usage.md)

## Workflows

| Workflow | Trigger | Result |
|---|---|---|
| [CI](../.github/workflows/ci.yml) | Pull request, push to `main`, manual dispatch, or a reusable-workflow call | Formatting, strict Clippy, API docs, workflow lint, debug and release tests |
| [Release](../.github/workflows/release.yml) | Push a `v*` tag or dispatch manually | CI first, then a Linux archive, checksum, and an extracted-binary smoke test |
| Release publication job | A version-tag run whose checks and build succeeded | GitHub Release with archive, checksum, and generated notes |

A manual Release run uploads build artifacts without creating a GitHub Release.
Its archive uses a `sha-<commit>` version. Version tags publish after validation;
tags containing a prerelease suffix create a GitHub prerelease.

All jobs run on Ubuntu 24.04 with the Rust version in
[`rust-toolchain.toml`](../rust-toolchain.toml). Workflow toolchain inputs use the
same explicit version. Update the file and both workflows together when upgrading,
then resolve new Clippy diagnostics before merging. The Rust edition and compiler
pin are separate choices; the pin is a tested build toolchain, not a declared MSRV.

The Actions dependencies are pinned to commit IDs. Cargo dependency versions come
from `Cargo.lock`, and each Cargo build/check/test uses `--locked`. CI caches the
Cargo registries, Git dependency cache, and target directory separately for each
job/profile. Cache keys include the lockfile and toolchain configuration. A cache
is only an optimization; a cold runner follows the same validation path.

## Validation gates

The quality job runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
scripts/check-workflows.sh
shellcheck scripts/*.sh
```

The test matrix runs the entire workspace twice:

```bash
cargo test --workspace --locked --profile test
cargo test --workspace --locked --profile release
```

Those invocations include library/unit tests, integration tests, CLI/server tests,
the screenshot baseline, and doctests. Release-profile testing checks the optimized
build as well as the default test profile. No external application service is
needed; some fixtures bind loopback HTTP/HTTPS listeners.

Workflow validation uses actionlint 1.7.12. The helper downloads its Linux x86_64
binary from the upstream release and checks a pinned SHA-256 digest before running
it. The helper needs Bash, curl, tar, and sha256sum. The GitHub-hosted runner also
provides ShellCheck, which actionlint uses when available. The release helpers
add Python 3.11+ for TOML parsing and PNG metadata checks.

Jobs have explicit timeouts. A newer commit cancels an older CI run for the same
workflow/ref. Release runs are serialized per ref and do not cancel an in-progress
publication. Pull-request validation has read-only repository access; the separate
publication job is the only job with `contents: write`. No custom repository
secret is required: publication uses GitHub's provided workflow token.

## New behavioral coverage

The tests added alongside the pipeline cover boundaries where stages cooperate:

- [`kernel/tests/lifecycle.rs`](../crates/kernel/tests/lifecycle.rs): edited values
  visible to input/change handlers, script-driven layout updates, mouse-event
  order, focus pixels, same-document fragment activation, microtask/timer order,
  and document-handle reuse without stale author styles.
- [`net/tests/response.rs`](../crates/net/tests/response.rs): repeated and mixed-case
  headers, declared Windows-1252 text, BOM/lossy UTF-8 behavior, binary base64 across
  padding boundaries, and exact decoded-body limits for data URLs.
- [`sol/tests/cli.rs`](../crates/sol/tests/cli.rs): screenshot output failures and
  load failures in addition to path resolution, script output, PNGs, and exit codes.

These complement the existing codec, TLS, cache, layout, font, paint, parser, and
JavaScript suites. The [development guide](development.md) maps the remaining tests
to their source modules.

## Build and inspect an archive locally

The initial release target is `x86_64-unknown-linux-gnu`. These helpers require a
Linux x86_64 build host with the target/toolchain installed. The executable links
against the host's GNU/Linux system libraries; this is not a static, universal
Linux, macOS, or Windows distribution. GitHub builds use Ubuntu 24.04.

From the repository root:

```bash
scripts/package-release.sh "sha-$(git rev-parse HEAD)"
scripts/verify-release.sh "dist/sol-sha-$(git rev-parse HEAD)-x86_64-unknown-linux-gnu.tar.gz"
```

`package-release.sh` validates its version before building, runs a locked optimized
build, and writes the archive plus a `.sha256` sidecar into ignored `dist/`.
For a version beginning with `v`, its version must exactly equal the `sol` package
version in `crates/sol/Cargo.toml`. Supported version labels are `vMAJOR.MINOR.PATCH`,
optionally followed by a prerelease suffix, or `sha-` followed by 7–40 hex digits.

The archive contains one versioned top-level directory with:

```text
bin/sol
LICENSE
README.md
licenses/LICENSE-DejaVu.txt
docs/
demo/
```

The verifier checks the sidecar checksum, extracts into a temporary directory,
checks the required files, and executes that extracted binary. It renders the
packaged demo at 320×200, checks PNG dimensions/signature, serializes a data-URL
page, and runs the load-stats command. Running from the archive catches errors
that tests against `target/` alone would not cover. Temporary staging directories
are removed on exit.

## Publish a version

1. Set the intended package versions in the crate manifests. Keep the executable's
   version aligned with the tag and commit the version/lockfile changes.
2. Ensure CI succeeds for that commit.
3. Create an annotated tag using the required identity, then push it:

   ```bash
   git -c user.name="lemonhub-io" -c user.email="lemonhub.io@icloud.com" \
     tag -a v0.1.0 -m "sol v0.1.0"
   git push origin v0.1.0
   ```

4. Open the Release workflow run. It reruns validation before packaging and
   publication; a failed check prevents the publication job from running.
5. Download the archive and its checksum from the GitHub Release. From their
   containing directory, verify with `sha256sum --check <archive>.sha256`.

The tag above is an example, not a tag created by installing these workflows.
A manual dispatch is the way to exercise delivery without publishing a version.
Workflow artifacts are retained for 14 days; published release assets remain
separate from that temporary artifact retention policy.

The pipeline does not publish crates to crates.io, deploy a web service, or create
a container image. Delivery for this command-line browser is a tested executable
archive in GitHub Releases. Additional targets should get their own native build,
test, archive format, and smoke checks before being advertised as supported.

## Inspect or rerun automation

Use the repository's **Actions** tab or GitHub CLI:

```bash
gh run list --workflow ci.yml
gh workflow run release.yml --ref main
gh run list --workflow release.yml
gh run view <run-id> --log-failed
gh run rerun <run-id> --failed
```

If the repository disables Actions or restricts which actions can run, those
settings must permit the pinned actions before jobs can start. A failing workflow
linter should be fixed in the workflow file rather than bypassed. A version/tag
mismatch should be resolved by correcting the intended release inputs, not by
shipping an archive with misleading version metadata.
