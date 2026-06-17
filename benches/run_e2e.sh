#!/usr/bin/env bash
# Run e2e_bench.rb for removed/shirobai modes, interleaved N rounds,
# then aggregate with aggregate_e2e.rb.
#
# Usage: benches/run_e2e.sh [rounds]
#        STOCK_FIXED=41.6 benches/run_e2e.sh 5
set -euo pipefail

rounds="${1:-3}"
here="$(cd "$(dirname "$0")" && pwd)"
out="$(mktemp)"
trap 'rm -f "$out"' EXIT

for _ in $(seq "$rounds"); do
  for mode in removed shirobai; do
    ruby "$here/e2e_bench.rb" "$mode"
  done
done | tee "$out"

STOCK_FIXED="${STOCK_FIXED:-40.0}" ruby "$here/aggregate_e2e.rb" "$out"
