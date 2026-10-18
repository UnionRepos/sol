#!/usr/bin/env bash
set -euo pipefail

archive="${1:?Usage: scripts/verify-release.sh <archive.tar.gz>}"
archive_dir="$(cd -- "$(dirname -- "$archive")" && pwd)"
archive_name="$(basename -- "$archive")"
(
  cd "$archive_dir"
  sha256sum --check "$archive_name.sha256"
)
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
tar -xzf "$archive_dir/$archive_name" -C "$staging"
package="$staging/${archive_name%.tar.gz}"
for required in bin/sol LICENSE README.md licenses/LICENSE-DejaVu.txt docs/usage.md demo/page.html; do
  test -f "$package/$required"
done
# Exercising the extracted executable catches missing assets and packaging errors
# that running the source-tree binary would leave invisible.
"$package/bin/sol" shot "$package/demo/page.html" "$staging/demo.png" 320x200
"$package/bin/sol" dump 'data:text/html,<title>Release smoke</title><p>ready</p>' > "$staging/dom.html"
"$package/bin/sol" check 'data:text/html,<title>Release smoke</title><p>ready</p>'
python3 - "$staging" <<'PY'
import pathlib
import struct
import sys

folder = pathlib.Path(sys.argv[1])
png = (folder / 'demo.png').read_bytes()
assert png[:8] == b'\x89PNG\r\n\x1a\n'
assert png[12:16] == b'IHDR'
assert struct.unpack('>II', png[16:24]) == (320, 200)
assert '<p>ready</p>' in (folder / 'dom.html').read_text()
PY
echo 'Packaged executable passed screenshot, DOM, and load checks.'
