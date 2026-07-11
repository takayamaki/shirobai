#!/usr/bin/env bash
# Compare two rubocop JSON outputs for per-cop / per-offense parity.
#
# Usage:
#   benches/parity_compare.sh <stock.json> <shirobai.json>
#
# Exits 0 on full parity, 1 on divergence.
set -euo pipefail

stock_json="${1:?usage: parity_compare.sh <stock.json> <shirobai.json>}"
sh_json="${2:?usage: parity_compare.sh <stock.json> <shirobai.json>}"

[[ -f "$stock_json" ]] || { echo "error: $stock_json not found" >&2; exit 1; }
[[ -f "$sh_json" ]] || { echo "error: $sh_json not found" >&2; exit 1; }

ruby -rjson <<RUBY
# Compare the FULL offense-key multiset, not just per-cop counts (key is
# path|cop|start:col|last:col|severity|correctable|message), so an equal-count
# difference in range / severity / correctable / message is caught too.
def load(path)
  d = JSON.parse(File.read(path))
  h = Hash.new(0); per = {}
  d["files"].each do |f|
    fp = f["path"]
    f["offenses"].each do |o|
      cop = o["cop_name"]
      l = o["location"]
      key = [fp, cop,
             "#{l["start_line"]}:#{l["start_column"]}",
             "#{l["last_line"]}:#{l["last_column"]}",
             o["severity"], o["correctable"], o["message"]].join("|")
      h[cop] += 1
      (per[cop] ||= Hash.new(0))[key] += 1
    end
  end
  [d["files"].size, d["summary"]["offense_count"], h, per]
end
st_files, st_total, st_h, st_per = load("$stock_json")
sh_files, sh_total, sh_h, sh_per = load("$sh_json")
puts format("stock    files=%-6d offenses=%d", st_files, st_total)
puts format("shirobai files=%-6d offenses=%d", sh_files, sh_total)
puts format("total diff: %+d", sh_total - st_total)
cops = (st_h.keys + sh_h.keys).uniq.sort
diffs = cops.reject { |c| (st_per[c] || {}) == (sh_per[c] || {}) }
if diffs.empty?
  puts "per-cop divergence: NONE (full parity)"
else
  puts "per-cop divergence (#{diffs.size} cops):"
  samples = []
  diffs.each do |c|
    stk = st_per[c] || {}; shk = sh_per[c] || {}
    only_sh = 0; only_st = 0
    (stk.keys | shk.keys).each do |k|
      delta = (shk[k] || 0) - (stk[k] || 0)
      only_sh += delta if delta > 0
      only_st += -delta if delta < 0
    end
    printf "  %-50s shirobai=%-6d stock=%-6d  diff=%+d  (sh-only=%d, st-only=%d)\n",
           c, sh_h[c], st_h[c], sh_h[c] - st_h[c], only_sh, only_st
    (shk.keys - stk.keys).each { |k| samples << "  sh-only: #{k}" }
    (stk.keys - shk.keys).each { |k| samples << "  st-only: #{k}" }
  end
  unless samples.empty?
    puts
    puts "sample diverging offense keys (#{[samples.size, 8].min} of #{samples.size}):"
    puts samples.first(8)
  end
  puts
  puts "Use the JSON files for per-offense inspection."
  exit 1
end
RUBY
