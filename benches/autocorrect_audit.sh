#!/usr/bin/env bash
# Corpus-level autocorrect byte audit — the truth oracle for `-a` drop-in compatibility.
#
# Copies the corpus twice (cp -rL, so symlinked corpora such as rubocop_source are
# materialized), runs stock rubocop -a on one copy and shirobai-enabled rubocop -a
# on the other, then byte-compares the resulting trees with diff -rq.
# Lint-mode parity (parity_diff.sh) cannot see divergences that only materialize
# as corrections cascade across -a iterations (PR #67 fixed 29 files' worth that
# had been invisible to every other gate) — this audit is the only gate that can.
#
# Usage:
#   benches/autocorrect_audit.sh <corpus-path> [work-dir]
#
# Examples:
#   benches/autocorrect_audit.sh .tmp/fluentd
#   benches/autocorrect_audit.sh .tmp/rubocop_source /tmp/aa-rc
#
# TargetRubyVersion is pinned to 3.1 via RUBOCOP_TARGET_RUBY_VERSION: the corpora
# carry no .ruby-version, and an unpinned run falls back to the 2.7 parser, which
# rejects some corpus files outright. Override with TRV=<ver> when needed.
# The two -a runs are sequential on purpose — paired-measurement discipline and
# CPU contention both forbid running the arms concurrently.
#
# The offense counts printed by the two arms may differ by a few: -a iterates
# until convergence and intermediate cascade states are not required to match.
# The gate is the final tree byte diff only; single-pass detection parity is
# parity_diff.sh's job.
#
# On byte parity the work dir is removed; on divergence it is kept for inspection.
# Set KEEP=1 to keep it either way.
#
# Requires Gemfile.stock and Gemfile.with_shirobai at repo root.
# Build shirobai first: `bundle exec rake compile`.
set -euo pipefail

corpus="${1:?usage: autocorrect_audit.sh <corpus-path> [work-dir]}"
root="$(cd "$(dirname "$0")/.." && pwd)"
trv="${TRV:-3.1}"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }
[[ -f "$root/Gemfile.stock" && -f "$root/Gemfile.with_shirobai" ]] \
  || { echo "error: Gemfile.stock / Gemfile.with_shirobai missing at $root" >&2; exit 1; }

corpus="$(cd "$corpus" && pwd)"
name="$(basename "$corpus")"
work="${2:-$(mktemp -d "/tmp/autocorrect_audit_${name}.XXXX")}"
mkdir -p "$work"

echo "=== copying $name twice into $work (cp -rL) ==="
cp -rL "$corpus" "$work/stock"
cp -rL "$corpus" "$work/shirobai"

run_arm() { # <gemfile> <tree> [extra rubocop args...]
  local gemfile="$1" tree="$2"; shift 2
  RUBOCOP_TARGET_RUBY_VERSION="$trv" BUNDLE_GEMFILE="$root/$gemfile" \
    bundle exec rubocop "$@" -a --force-default-config --cache false --no-server \
    "$tree" > "$work/$(basename "$tree").log" 2>&1 || true # non-zero on offenses
  tail -1 "$work/$(basename "$tree").log"
}

echo
echo "=== stock -a on copy (TRV=$trv) ==="
time run_arm Gemfile.stock "$work/stock"

echo
echo "=== shirobai -a on copy (TRV=$trv) ==="
time run_arm Gemfile.with_shirobai "$work/shirobai" --require shirobai

echo
echo "=== tree byte diff ==="
if diff -rq --no-dereference --exclude=rubocop_cache "$work/stock" "$work/shirobai" > "$work/tree.diff"; then
  echo "BYTE PARITY: zero differing files ($name)"
  [[ "${KEEP:-0}" = "1" ]] || rm -rf "$work"
  exit 0
else
  echo "DIVERGENCE (${name}): $(wc -l < "$work/tree.diff") diff lines"
  head -20 "$work/tree.diff"
  echo
  echo "Full diff at $work/tree.diff; corrected trees kept at $work for inspection."
  exit 1
fi
