#!/usr/bin/env bash
# Real-CLI dual-Gemfile parity diff — the truth oracle for drop-in compatibility.
#
# Runs stock rubocop (Gemfile.stock) and shirobai-enabled rubocop
# (Gemfile.with_shirobai + --require shirobai) over the same corpus, both with
# --force-default-config --cache false --no-server -f json, then diffs the two
# offense sets exactly: per-cop counts, per-offense path/line/column/message.
#
# Usage:
#   benches/parity_diff.sh <corpus-path> [out-prefix]
#
# Examples:
#   benches/parity_diff.sh .tmp/mastodon
#   benches/parity_diff.sh .tmp/discourse /tmp/dc
#   benches/parity_diff.sh .tmp/rubocop_source /tmp/rc
#
# Requires Gemfile.stock and Gemfile.with_shirobai at repo root.
# Build shirobai first: `bundle exec rake compile`.
set -euo pipefail

corpus="${1:?usage: parity_diff.sh <corpus-path> [out-prefix]}"
prefix="${2:-/tmp/parity}"
root="$(cd "$(dirname "$0")/.." && pwd)"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }
[[ -f "$root/Gemfile.stock" && -f "$root/Gemfile.with_shirobai" ]] \
  || { echo "error: Gemfile.stock / Gemfile.with_shirobai missing at $root" >&2; exit 1; }

stock_json="${prefix}_stock.json"
sh_json="${prefix}_shirobai.json"

echo "=== stock real-CLI on $corpus ==="
time BUNDLE_GEMFILE="$root/Gemfile.stock" bundle exec rubocop \
  --force-default-config --cache false --no-server -f json \
  "$corpus" > "$stock_json" 2>/dev/null || true   # rubocop exits non-zero on offenses

echo
echo "=== shirobai real-CLI on $corpus ==="
time BUNDLE_GEMFILE="$root/Gemfile.with_shirobai" bundle exec rubocop \
  --require shirobai --force-default-config --cache false --no-server -f json \
  "$corpus" > "$sh_json" 2>/dev/null || true

echo
echo "=== summary ==="
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
  puts "Use the JSON files at $stock_json / $sh_json for per-offense inspection."
end
RUBY
