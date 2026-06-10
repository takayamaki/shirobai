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
cpu     = Hash.new { |h, k| h[k] = [] }
compute = Hash.new { |h, k| h[k] = [] }
gc      = Hash.new { |h, k| h[k] = [] }
wall    = Hash.new { |h, k| h[k] = [] }
offenses = {}
File.foreach(ARGV[0]) do |line|
  next unless line =~ /^(\w+)\b.*offenses=(\d+)\s+cpu=(\d+\.\d+)s\s+compute=(\d+\.\d+)s\s+gc=(\d+\.\d+)s\s+wall=(\d+\.\d+)s/
  offenses[$1] = $2.to_i
  cpu[$1]     << $3.to_f
  compute[$1] << $4.to_f
  gc[$1]      << $5.to_f
  wall[$1]    << $6.to_f
end
median = lambda do |a|
  return 0.0 if a.empty?
  b = a.sort
  n = b.size
  n.odd? ? b[n / 2] : (b[n / 2 - 1] + b[n / 2]) / 2.0
end
lo = ->(a) { a.empty? ? 0.0 : a.min }
hi = ->(a) { a.empty? ? 0.0 : a.max }
# `compute = cpu - gc` is the low-variance headline: GC time is the dominant
# run-to-run noise. We report the net from BOTH compute (clean) and cpu (total,
# GC-inclusive), plus gc on its own (shirobai allocates less, so lower gc is a
# real part of its win). min/max are the compute spread.
sc = median.call(compute["stock"]); rc = median.call(compute["removed"]); hc = median.call(compute["shirobai"])
st = median.call(cpu["stock"]);     ht = median.call(cpu["shirobai"])
puts
printf("           compute med  (min .. max)      gc med    cpu med   wall med  offenses\n")
%w[stock removed shirobai].each do |m|
  printf("%-9s  %7.2fs    (%5.2f .. %5.2f)   %6.2fs   %7.2fs  %7.2fs  %d\n",
         m, median.call(compute[m]), lo.call(compute[m]), hi.call(compute[m]),
         median.call(gc[m]), median.call(cpu[m]), median.call(wall[m]), offenses[m] || 0)
end
puts
parity = offenses["stock"] == offenses["shirobai"] ? "OK (= stock)" : "MISMATCH vs stock=#{offenses["stock"]}"
printf("offense parity:               %s\n", parity)
printf("replaced cops Ruby compute:   %+.2fs  (stock - removed, compute median)\n", sc - rc)
printf("replaced cops Rust  compute:  %+.2fs  (shirobai - removed, compute median)\n", hc - rc)
printf("net win (compute, clean):     %+.2fs  (%.1f%%)  (stock - shirobai)\n", sc - hc, sc.zero? ? 0 : (sc - hc) / sc * 100)
printf("net win (cpu, GC-inclusive):  %+.2fs  (%.1f%%)  (stock - shirobai)\n", st - ht, st.zero? ? 0 : (st - ht) / st * 100)
printf("gc: stock %.2fs / shirobai %.2fs (shirobai allocates less)\n",
       median.call(gc["stock"]), median.call(gc["shirobai"]))
ws = median.call(wall["stock"])
if ws.positive? && st / ws < 0.85
  printf("\n[warn] stock cpu/wall = %.2f (<0.85): external load contaminated wall time; trust cpu/compute.\n", st / ws)
end
' "$out"
