#!/usr/bin/env bash
# Real-CLI benchmark for README publication.
# Runs stock and shirobai rubocop on a corpus using its own .rubocop.yml,
# with all plugin gems installed via benches/Gemfile.realconfig.{stock,shirobai}.
# No --force-default-config: this measures what a user would actually experience.
#
# Usage: benches/real_cli_bench.sh <corpus-path> [rounds]
# Example: benches/real_cli_bench.sh .tmp/mastodon 3
#
# Environment variables:
#   VERIFY=1        Before the timed rounds, run each mode once with JSON output
#                   and fail when the offense sets differ (benches/offense_diff.rb).
#                   These runs also warm the file cache before timing starts.
#   SUMMARY_FILE=f  Append a markdown summary to f (use $GITHUB_STEP_SUMMARY on CI).
set -euo pipefail

corpus="${1:?usage: real_cli_bench.sh <corpus-path> [rounds]}"
rounds="${2:-3}"
root="$(cd "$(dirname "$0")/.." && pwd)"
benchdir="$root/benches"
corpus_name="$(basename "$corpus")"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }

echo "corpus: $corpus  rounds: $rounds"
echo

run_rubocop() {
  local mode="$1"; shift
  local gemfile extra_args
  if [ "$mode" = "stock" ]; then
    gemfile="$benchdir/Gemfile.realconfig.stock"
    extra_args=""
  else
    gemfile="$benchdir/Gemfile.realconfig.shirobai"
    extra_args="--require shirobai"
  fi
  BUNDLE_GEMFILE="$gemfile" bundle exec rubocop \
    $extra_args --cache false --no-server "$@" "$corpus"
}

offenses="" files=""
if [ "${VERIFY:-0}" = "1" ]; then
  workdir="$(mktemp -d)"
  echo "=== verify: offense sets must match (also warms the file cache) ==="
  for mode in stock shirobai; do
    echo "verify run: $mode"
    status=0
    run_rubocop "$mode" -f json -o "$workdir/$mode.json" > "$workdir/$mode.log" 2>&1 || status=$?
    # rubocop exits 0 (clean) or 1 (offenses); 2+ means it did not run
    if [ "$status" -ge 2 ]; then
      echo "error: $mode verify run failed (exit $status):" >&2
      cat "$workdir/$mode.log" >&2
      exit 1
    fi
  done
  ruby "$benchdir/offense_diff.rb" "$workdir/stock.json" "$workdir/shirobai.json"
  offenses=$(ruby -rjson -e 'puts JSON.parse(File.read(ARGV[0]))["summary"]["offense_count"]' "$workdir/stock.json")
  files=$(ruby -rjson -e 'puts JSON.parse(File.read(ARGV[0]))["files"].size' "$workdir/stock.json")
  echo
fi

results=""

for i in $(seq "$rounds"); do
  for mode in stock shirobai; do
    elapsed=$( { time run_rubocop "$mode" -f quiet > /dev/null 2>&1 || true; } 2>&1 | grep real | awk '{print $2}' )

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
printf "$results" | CORPUS_NAME="$corpus_name" ROUNDS="$rounds" \
  OFFENSES="$offenses" FILES="$files" SUMMARY_FILE="${SUMMARY_FILE:-}" ruby -e '
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

summary = ENV["SUMMARY_FILE"].to_s
unless summary.empty?
  File.open(summary, "a") do |f|
    f.puts "### #{ENV["CORPUS_NAME"]}"
    f.puts
    unless ENV["OFFENSES"].to_s.empty?
      f.puts "Verified: stock and shirobai report the same #{ENV["OFFENSES"]} offenses" \
             " over #{ENV["FILES"]} files (corpus config)."
      f.puts
    end
    f.puts "| rounds | stock median | shirobai median | saving |"
    f.puts "|---|---|---|---|"
    f.puts format("| %s | %.2fs | %.2fs | **%.2fs (%.1f%%)** |",
                  ENV["ROUNDS"], sm, hm, saving, pct)
    f.puts
  end
end
'
