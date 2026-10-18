#!/usr/bin/env bash
set -euo pipefail

version="${1:?Usage: scripts/package-release.sh <vVERSION|sha-COMMIT>}"
if [[ ! "$version" =~ ^(v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z][0-9A-Za-z.-]*)?|sha-[0-9a-f]{7,40})$ ]]; then
  echo "Invalid release version: $version" >&2
  exit 2
fi
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

# A tag and its executable must identify the same release; accepting a mismatch
# would leave users unable to relate a downloaded binary to its package version.
python3 - "$version" <<'PY'
import pathlib
import sys
import tomllib

version = sys.argv[1]
package = tomllib.loads(pathlib.Path('crates/sol/Cargo.toml').read_text())['package']
if version.startswith('v') and version[1:] != package['version']:
    sys.exit(f"Tag {version} does not match sol version {package['version']}")
PY

target=x86_64-unknown-linux-gnu
host="$(rustc --version --verbose | sed -n 's/^host: //p')"
if [[ "$host" != "$target" ]]; then
  echo "Release packaging requires a $target host, found $host" >&2
  exit 2
fi
cargo build --locked --release -p sol
name="sol-${version}-${target}"
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/$name/bin" "$staging/$name/licenses" dist
cp target/release/sol "$staging/$name/bin/sol"
cp LICENSE README.md "$staging/$name/"
cp crates/text/assets/LICENSE-DejaVu.txt "$staging/$name/licenses/"
cp -R docs demo "$staging/$name/"
tar -czf "dist/$name.tar.gz" -C "$staging" "$name"
(
  cd dist
  sha256sum "$name.tar.gz" > "$name.tar.gz.sha256"
)
echo "Created dist/$name.tar.gz and its SHA-256 checksum."
