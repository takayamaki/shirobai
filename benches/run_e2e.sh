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
cpu  = Hash.new { |h, k| h[k] = [] }
wall = Hash.new { |h, k| h[k] = [] }
offenses = {}
File.foreach(ARGV[0]) do |line|
  next unless line =~ /^(\w+)\b.*offenses=(\d+)\s+cpu=(\d+\.\d+)s\s+wall=(\d+\.\d+)s/
  offenses[$1] = $2.to_i
  cpu[$1]  << $3.to_f
  wall[$1] << $4.to_f
end
median = lambda do |a|
  return 0.0 if a.empty?
  b = a.sort
  n = b.size
  n.odd? ? b[n / 2] : (b[n / 2 - 1] + b[n / 2]) / 2.0
end
lo = ->(a) { a.empty? ? 0.0 : a.min }
hi = ->(a) { a.empty? ? 0.0 : a.max }
# Headline metric is median CPU time (robust to outliers, and representative of
# the GC-inclusive cost we deliberately measure). min/max show the spread; the
# net win is computed from medians. Wall median is a sanity check only.
s = median.call(cpu["stock"]); r = median.call(cpu["removed"]); h = median.call(cpu["shirobai"])
puts
printf("           cpu median   (min .. max)        wall med   offenses\n")
%w[stock removed shirobai].each do |m|
  printf("%-9s  %7.2fs     (%5.2f .. %5.2f)     %7.2fs   %d\n",
         m, median.call(cpu[m]), lo.call(cpu[m]), hi.call(cpu[m]), median.call(wall[m]), offenses[m] || 0)
end
puts
parity = offenses["stock"] == offenses["shirobai"] ? "OK (= stock)" : "MISMATCH vs stock=#{offenses["stock"]}"
printf("offense parity:           %s\n", parity)
printf("replaced cops Ruby eval:  %+.2fs  (stock - removed, cpu median)\n", s - r)
printf("replaced cops Rust cost:  %+.2fs  (shirobai - removed, cpu median)\n", h - r)
printf("net win:                  %+.2fs  (%.1f%%)  (stock - shirobai, cpu median)\n", s - h, s.zero? ? 0 : (s - h) / s * 100)
# Sanity: if cpu is much smaller than wall, external load inflated the run.
ws = median.call(wall["stock"])
if ws.positive? && s / ws < 0.85
  printf("\n[warn] stock cpu/wall = %.2f (<0.85): external load contaminated wall time; trust cpu.\n", s / ws)
end
' "$out"
