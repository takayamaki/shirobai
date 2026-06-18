#!/usr/bin/env bash
# Real-CLI benchmark for README publication.
# Runs stock and shirobai rubocop on a corpus using its own .rubocop.yml,
# with all plugin gems installed via benches/Gemfile.realconfig.{stock,shirobai}.
# No --force-default-config: this measures what a user would actually experience.
#
# Usage: benches/real_cli_bench.sh <corpus-path> [rounds]
# Example: benches/real_cli_bench.sh .tmp/mastodon 3
set -euo pipefail

corpus="${1:?usage: real_cli_bench.sh <corpus-path> [rounds]}"
rounds="${2:-3}"
root="$(cd "$(dirname "$0")/.." && pwd)"
benchdir="$root/benches"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }

echo "corpus: $corpus  rounds: $rounds"
echo

results=""

for i in $(seq "$rounds"); do
  for mode in stock shirobai; do
    if [ "$mode" = "stock" ]; then
      gemfile="$benchdir/Gemfile.realconfig.stock"
      extra_args=""
    else
      gemfile="$benchdir/Gemfile.realconfig.shirobai"
      extra_args="--require shirobai"
    fi

    elapsed=$( { time BUNDLE_GEMFILE="$gemfile" bundle exec rubocop \
      $extra_args --cache false --no-server -f quiet \
      "$corpus" > /dev/null 2>&1 || true; } 2>&1 | grep real | awk '{print $2}' )

    secs=$(echo "$elapsed" | ruby -e '
      m = gets.strip.match(/(\d+)m([\d.]+)s/)
      puts m[1].to_f * 60 + m[2].to_f
    ')

    printf "round=%d  %-9s  %s  (%.2fs)\n" "$i" "$mode" "$elapsed" "$secs"
    results="$results$mode $secs\n"
  done
done

echo
echo "=== summary ==="
printf "$results" | ruby -e '
vals = Hash.new { |h,k| h[k] = [] }
ARGF.each_line do |l|
  mode, t = l.strip.split
  next unless mode && t
  vals[mode] << t.to_f
end

median = ->(a) {
  b = a.sort; n = b.size
  n.odd? ? b[n/2] : (b[n/2-1] + b[n/2]) / 2.0
}

sm = median.call(vals["stock"])
hm = median.call(vals["shirobai"])
saving = sm - hm
pct = sm.zero? ? 0 : saving / sm * 100

printf "stock    median: %.2fs  (min %.2f, max %.2f)\n", sm, vals["stock"].min, vals["stock"].max
printf "shirobai median: %.2fs  (min %.2f, max %.2f)\n", hm, vals["shirobai"].min, vals["shirobai"].max
printf "saving:  %.2fs  (%.1f%%)\n", saving, pct
'
