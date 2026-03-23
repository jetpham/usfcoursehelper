#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  printf 'usage: %s <repository> <tag> <asset-name> <sha256> <output-path>\n' "$0" >&2
  exit 1
fi

repository="$1"
tag="$2"
asset_name="$3"
sha256="$4"
output_path="$5"
version="${tag#v}"

cat > "$output_path" <<EOF
class Usfcoursehelper < Formula
  desc "Scrape USF course sections into CSV and calendar files"
  homepage "https://github.com/${repository}"
  url "https://github.com/${repository}/releases/download/${tag}/${asset_name}"
  sha256 "${sha256}"
  version "${version}"

  def install
    bin.install "usfcoursehelper"
    doc.install "README.md" if File.exist?("README.md")
  end

  test do
    assert_match "usfcoursehelper", shell_output("#{bin}/usfcoursehelper --help")
  end
end
EOF
