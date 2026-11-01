#!/usr/bin/env bash
# Make a CRLF copy of a corpus for the CRLF parity gate.
#
# parser-gem normalizes "\r\n" to "\n" in `buffer.source`, while
# `ProcessedSource#raw_source` keeps the "\r". A wrapper that maps Rust byte
# offsets from `raw_source` reports wrong positions on such files, and the
# default corpora are all LF, so `parity_diff.sh` never sees it.
# This script copies the corpus and rewrites every file rubocop inspects
# (`rubocop -L`) to CRLF line endings.
#
# Usage:
#   benches/crlf_corpus.sh <corpus-path> <copy-path>
#   benches/parity_diff.sh <copy-path> [out-prefix]
#
# Example:
#   benches/crlf_corpus.sh .tmp/fluentd .tmp/fluentd_crlf
#   benches/parity_diff.sh .tmp/fluentd_crlf /tmp/fluentd_crlf
#
# The copy is recreated from scratch on every run.
set -euo pipefail

src="${1:?usage: crlf_corpus.sh <corpus-path> <copy-path>}"
dst="${2:?usage: crlf_corpus.sh <corpus-path> <copy-path>}"
root="$(cd "$(dirname "$0")/.." && pwd)"

[[ -d "$src" ]] || { echo "error: $src does not exist" >&2; exit 1; }

rm -rf "$dst"
mkdir -p "$(dirname "$dst")"
cp -R "$src" "$dst"

dst="$(cd "$dst" && pwd)"

# List the files the same way parity_diff.sh runs rubocop: from the repo root
# with the corpus path as the argument. Default Exclude patterns anchor to the
# working directory, so listing from inside the copy would pick other files.
list="$(mktemp)"
trap 'rm -f "$list"' EXIT
(cd "$root" && BUNDLE_GEMFILE="$root/Gemfile.stock" bundle exec rubocop \
  --force-default-config --no-server -L "$dst" 2>/dev/null) > "$list"

count=0
while IFS= read -r file; do
  [[ "$file" = /* ]] || file="$root/$file"
  perl -pi -e 's/\r?\n/\r\n/' "$file"
  count=$((count + 1))
done < "$list"

echo "converted $count files to CRLF in $dst"
