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
def load(path)
  d = JSON.parse(File.read(path))
  h = Hash.new(0); per = {}
  d["files"].each do |f|
    fp = f["path"]
    f["offenses"].each do |o|
      cop = o["cop_name"]
      key = "#{fp}|#{o["location"]["line"]}:#{o["location"]["column"]}|#{o["message"]}"
      h[cop] += 1
      (per[cop] ||= {})[key] = true
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
diffs = cops.reject { |c| sh_h[c] == st_h[c] }
if diffs.empty?
  puts "per-cop divergence: NONE (full parity)"
else
  puts "per-cop divergence (#{diffs.size} cops):"
  diffs.each do |c|
    only_sh = ((sh_per[c]||{}).keys - (st_per[c]||{}).keys).size
    only_st = ((st_per[c]||{}).keys - (sh_per[c]||{}).keys).size
    printf "  %-50s shirobai=%-6d stock=%-6d  diff=%+d  (sh-only=%d, st-only=%d)\n",
           c, sh_h[c], st_h[c], sh_h[c] - st_h[c], only_sh, only_st
  end
  puts
  puts "Use the JSON files for per-offense inspection."
  exit 1
end
RUBY
