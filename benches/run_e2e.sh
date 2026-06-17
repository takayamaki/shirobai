#!/usr/bin/env bash
# Run e2e_bench.rb for stock/removed/shirobai modes, interleaved N rounds,
# then aggregate with aggregate_e2e.rb.
#
# Uses the corpus's own .rubocop.yml for config.
#
# Usage: benches/run_e2e.sh [corpus-path] [rounds]
#        benches/run_e2e.sh .tmp/mastodon 3
set -euo pipefail

corpus="${1:-.tmp/mastodon}"
rounds="${2:-3}"
here="$(cd "$(dirname "$0")" && pwd)"
out="$(mktemp)"
trap 'rm -f "$out"' EXIT

echo "corpus: $corpus  rounds: $rounds"
echo

for _ in $(seq "$rounds"); do
  for mode in stock removed shirobai; do
    ruby "$here/e2e_bench.rb" "$mode" "$corpus"
  done
done | tee "$out"

ruby "$here/aggregate_e2e.rb" "$out"
