#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

tool_dir="$(mktemp -d)"
trap 'rm -rf "$tool_dir"' EXIT
version=1.7.12
archive="actionlint_${version}_linux_amd64.tar.gz"
# The expected digest stays with the version so a damaged download cannot decide
# which workflow definitions are accepted by CI.
curl --fail --silent --show-error --location \
  "https://github.com/rhysd/actionlint/releases/download/v${version}/${archive}" \
  --output "$tool_dir/$archive"
(
  cd "$tool_dir"
  echo "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8  $archive" | sha256sum --check
)
tar -xzf "$tool_dir/$archive" -C "$tool_dir" actionlint
"$tool_dir/actionlint" -color
