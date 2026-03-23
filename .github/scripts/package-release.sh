#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 4 || $# -gt 5 ]]; then
  printf 'usage: %s <binary-path> <binary-name> <package-stem> <output-dir> [readme-path]\n' "$0" >&2
  exit 1
fi

binary_path="$1"
binary_name="$2"
package_stem="$3"
output_dir="$4"
readme_path="${5:-}"

stage_root="$(mktemp -d)"
package_dir="$stage_root/$package_stem"
mkdir -p "$package_dir" "$output_dir"

cp "$binary_path" "$package_dir/$binary_name"

if [[ "$binary_name" != *.exe ]]; then
  chmod +x "$package_dir/$binary_name"
fi

raw_suffix=""
if [[ "$binary_name" == *.exe ]]; then
  raw_suffix=".exe"
fi

if [[ -n "$readme_path" && -f "$readme_path" ]]; then
  cp "$readme_path" "$package_dir/README.md"
fi

cp "$package_dir/$binary_name" "$output_dir/$package_stem$raw_suffix"

tar -C "$stage_root" -czf "$output_dir/$package_stem.tar.gz" "$package_stem"
tar -C "$stage_root" -cJf "$output_dir/$package_stem.tar.xz" "$package_stem"
(
  cd "$stage_root"
  zip -qr "$output_dir/$package_stem.zip" "$package_stem"
)

rm -rf "$stage_root"
