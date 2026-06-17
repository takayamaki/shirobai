# frozen_string_literal: true

# Aggregates output from e2e_bench.rb runs (piped from run_e2e.sh).
# Reads lines like:
#   shirobai cops=394 replaced=63 files=3181 offenses=16846 cpu=25.90s compute=23.40s gc=2.50s wall=27.10s
#
# Usage:
#   cat results.txt | ruby benches/aggregate_e2e.rb
#   ruby benches/aggregate_e2e.rb results.txt

input = ARGF

cpu     = Hash.new { |h, k| h[k] = [] }
compute = Hash.new { |h, k| h[k] = [] }
gc      = Hash.new { |h, k| h[k] = [] }
wall    = Hash.new { |h, k| h[k] = [] }
offenses = {}

input.each_line do |line|
  next unless line =~ /^(\w+)\b.*offenses=(\d+)\s+cpu=(\d+\.\d+)s\s+compute=(\d+\.\d+)s\s+gc=(\d+\.\d+)s\s+wall=(\d+\.\d+)s/

  offenses[Regexp.last_match(1)] = Regexp.last_match(2).to_i
  cpu[Regexp.last_match(1)]     << Regexp.last_match(3).to_f
  compute[Regexp.last_match(1)] << Regexp.last_match(4).to_f
  gc[Regexp.last_match(1)]      << Regexp.last_match(5).to_f
  wall[Regexp.last_match(1)]    << Regexp.last_match(6).to_f
end

median = lambda do |a|
  return 0.0 if a.empty?

  b = a.sort
  n = b.size
  n.odd? ? b[n / 2] : (b[n / 2 - 1] + b[n / 2]) / 2.0
end
lo = ->(a) { a.empty? ? 0.0 : a.min }
hi = ->(a) { a.empty? ? 0.0 : a.max }

sc = median.call(compute["stock"])
rc = median.call(compute["removed"])
hc = median.call(compute["shirobai"])
st = median.call(cpu["stock"])
ht = median.call(cpu["shirobai"])

puts
printf("           compute med  (min .. max)      gc med    cpu med   wall med  offenses\n")
%w[stock removed shirobai].each do |m|
  printf("%-9s  %7.2fs    (%5.2f .. %5.2f)   %6.2fs   %7.2fs  %7.2fs  %d\n",
         m,
         median.call(compute[m]), lo.call(compute[m]), hi.call(compute[m]),
         median.call(gc[m]), median.call(cpu[m]), median.call(wall[m]),
         offenses[m] || 0)
end

puts
parity = if offenses["stock"] == offenses["shirobai"]
           "OK (stock = shirobai)"
         else
           "MISMATCH stock=#{offenses["stock"]} shirobai=#{offenses["shirobai"]}"
         end
printf("offense parity:               %s\n", parity)
printf("replaced cops Ruby compute:   %+.2fs  (stock - removed)\n", sc - rc)
printf("replaced cops Rust  compute:  %+.2fs  (shirobai - removed)\n", hc - rc)
printf("net win (compute, clean):     %+.2fs  (%.1f%%)  (stock - shirobai)\n",
       sc - hc, sc.zero? ? 0 : (sc - hc) / sc * 100)
printf("net win (cpu, GC-inclusive):  %+.2fs  (%.1f%%)  (stock - shirobai)\n",
       st - ht, st.zero? ? 0 : (st - ht) / st * 100)
printf("gc: stock %.2fs / shirobai %.2fs\n",
       median.call(gc["stock"]), median.call(gc["shirobai"]))

ws = median.call(wall["stock"])
if ws.positive? && st / ws < 0.85
  printf("\n[warn] stock cpu/wall = %.2f (<0.85): external load likely; trust cpu/compute.\n", st / ws)
end
