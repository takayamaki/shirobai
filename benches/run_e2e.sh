#!/usr/bin/env bash
# End-to-end benchmark across all three modes, each in a fresh process, the
# modes interleaved and repeated N rounds (default 3) to average out drift.
#
#   stock    - all 394 default cops, unchanged
#   removed  - the implemented cops dropped entirely (baseline: stock minus
#              those cops' Ruby evaluation cost)
#   shirobai - the implemented cops swapped for the Rust drop-ins
#
# The `removed` baseline isolates the economics of the replaced cops, which the
# raw stock-vs-shirobai difference buries under per-run noise:
#   replaced cops' Ruby eval cost = stock   - removed
#   replaced cops' Rust  cost     = shirobai - removed
#   net win                       = stock   - shirobai
#
# Usage: benches/run_e2e.sh [rounds]
set -euo pipefail

rounds="${1:-3}"
here="$(cd "$(dirname "$0")" && pwd)"
out="$(mktemp)"
trap 'rm -f "$out"' EXIT

for _ in $(seq "$rounds"); do
  for mode in stock removed shirobai; do
    ruby "$here/e2e_bench.rb" "$mode"
  done
done | tee "$out"

ruby -e '
times = Hash.new { |h, k| h[k] = [] }
offenses = {}
File.foreach(ARGV[0]) do |line|
  next unless line =~ /^(\w+)\b.*offenses=(\d+)\s+(\d+\.\d+)s/
  times[$1] << $3.to_f
  offenses[$1] = $2.to_i
end
mean = ->(a) { a.empty? ? 0.0 : a.sum / a.size }
s = mean.call(times["stock"]); r = mean.call(times["removed"]); h = mean.call(times["shirobai"])
puts
printf("                  mean    offenses\n")
printf("stock           %6.2fs   %d\n", s, offenses["stock"])
printf("removed         %6.2fs   %d\n", r, offenses["removed"])
printf("shirobai        %6.2fs   %d\n", h, offenses["shirobai"])
puts
parity = offenses["stock"] == offenses["shirobai"] ? "OK (= stock)" : "MISMATCH vs stock=#{offenses["stock"]}"
printf("offense parity:           %s\n", parity)
printf("replaced cops Ruby eval:  %+.2fs  (stock - removed)\n", s - r)
printf("replaced cops Rust cost:  %+.2fs  (shirobai - removed)\n", h - r)
printf("net win:                  %+.2fs  (%.1f%%)  (stock - shirobai)\n", s - h, s.zero? ? 0 : (s - h) / s * 100)
' "$out"
