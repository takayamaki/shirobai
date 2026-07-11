#!/usr/bin/env bash
# Real-CLI dual-Gemfile parity diff for the shirobai-performance plugin —
# the truth oracle for plugin-cop drop-in compatibility.
#
# Same protocol as parity_diff.sh (which stays the core oracle), with the
# rubocop-performance plugin loaded on BOTH sides:
#
#   stock    = rubocop + rubocop-performance      (Gemfile.stock.performance)
#   shirobai = + shirobai + shirobai-performance  (Gemfile.with_shirobai.performance)
#
# Both runs use `--plugin rubocop-performance` (merges the plugin's
# default.yml into the default configuration at CLI-option time, so
# `--force-default-config` keeps working) and `--enable-pending-cops`
# (rubocop-performance ships several `Enabled: pending` cops, e.g.
# Performance/StringInclude — without this flag they would never run and
# the oracle would not exercise them).
#
# Usage:
#   benches/parity_diff_performance.sh <corpus-path> [out-prefix]
#
# Examples:
#   benches/parity_diff_performance.sh .tmp/mastodon
#   benches/parity_diff_performance.sh .tmp/redmine /tmp/rm
#
# Build shirobai first: `bundle exec rake compile`.
set -euo pipefail

corpus="${1:?usage: parity_diff_performance.sh <corpus-path> [out-prefix]}"
prefix="${2:-/tmp/parity_perf}"
root="$(cd "$(dirname "$0")/.." && pwd)"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }
[[ -f "$root/Gemfile.stock.performance" && -f "$root/Gemfile.with_shirobai.performance" ]] \
  || { echo "error: performance oracle Gemfiles missing at $root" >&2; exit 1; }

stock_json="${prefix}_stock.json"
sh_json="${prefix}_shirobai.json"

echo "=== stock real-CLI (rubocop-performance) on $corpus ==="
time BUNDLE_GEMFILE="$root/Gemfile.stock.performance" bundle exec rubocop \
  --plugin rubocop-performance --enable-pending-cops \
  --force-default-config --cache false --no-server -f json \
  "$corpus" > "$stock_json" 2>/dev/null || true   # rubocop exits non-zero on offenses

echo
echo "=== shirobai real-CLI (shirobai-performance) on $corpus ==="
time BUNDLE_GEMFILE="$root/Gemfile.with_shirobai.performance" bundle exec rubocop \
  --plugin rubocop-performance --require shirobai-performance --enable-pending-cops \
  --force-default-config --cache false --no-server -f json \
  "$corpus" > "$sh_json" 2>/dev/null || true

echo
echo "=== summary ==="
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
  puts "Use the JSON files at $stock_json / $sh_json for per-offense inspection."
  exit 1
end
RUBY
