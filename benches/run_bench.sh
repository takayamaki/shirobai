#!/usr/bin/env bash
# Run the per-cop incremental benchmark for both stock and shirobai, each in a
# fresh process, repeated N times (default 3) to expose variance.
#
# Usage: benches/run_bench.sh "Lint/Debugger" [runs]
set -euo pipefail

cop_name="${1:?usage: run_bench.sh <Cop/Name> [runs]}"
runs="${2:-3}"
here="$(cd "$(dirname "$0")" && pwd)"

for mode in stock shirobai; do
  for _ in $(seq "$runs"); do
    ruby "$here/incremental_bench.rb" "$mode" "$cop_name"
  done
done
