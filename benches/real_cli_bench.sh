#!/usr/bin/env bash
# Real-CLI benchmark for README publication.
# Runs one or more modes of rubocop on a corpus using its own .rubocop.yml,
# with all plugin gems installed via benches/Gemfile.realconfig.{stock,shirobai,main}.
# No --force-default-config: this measures what a user would actually experience.
#
# Usage: benches/real_cli_bench.sh <corpus-path> [rounds]
# Example: benches/real_cli_bench.sh .tmp/mastodon 3
#
# Environment variables:
#   MODES="stock shirobai"   Space-separated modes to run, in this order, each
#                            round. Default is the historical "stock shirobai".
#                            Supported values:
#                              stock     - unchanged rubocop (Gemfile.realconfig.stock)
#                              shirobai  - rubocop + shirobai from this tree
#                              removed   - stock rubocop with the implemented cops
#                                          dropped via --except (theoretical upper
#                                          bound of the replacement: stock - removed)
#                              main      - rubocop + shirobai from a second checkout
#                                          of the main branch (Gemfile.realconfig.main).
#                                          Requires SHIROBAI_MAIN_TREE.
#   rounds=N                 Timed rounds (the positional [rounds] arg wins over this).
#   SHIROBAI_MAIN_TREE=dir   Checkout of shirobai's main branch (required by main).
#   VERIFY=1                 Before the timed rounds, run each mode once with JSON
#                            output and fail when a shirobai-family mode's offense
#                            set differs from stock (benches/offense_diff.rb).
#                            These runs also warm the file cache before timing.
#   SUMMARY_FILE=f           Append a markdown summary to f (use $GITHUB_STEP_SUMMARY).
set -euo pipefail

corpus="${1:?usage: real_cli_bench.sh <corpus-path> [rounds]}"
rounds="${2:-${rounds:-3}}"
root="$(cd "$(dirname "$0")/.." && pwd)"
benchdir="$root/benches"
corpus_name="$(basename "$corpus")"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }

# Parse and validate MODES (empty falls back to the historical default).
read -ra modes <<< "${MODES:-stock shirobai}"
for m in "${modes[@]}"; do
  case "$m" in
    stock|shirobai|removed|main) ;;
    *) echo "error: unknown mode '$m' (valid: stock shirobai removed main)" >&2; exit 1 ;;
  esac
done

modes_contains() {
  local needle="$1" m
  for m in "${modes[@]}"; do [[ "$m" == "$needle" ]] && return 0; done
  return 1
}

# main needs a second checkout; fail early and clearly if it is missing.
if modes_contains main; then
  : "${SHIROBAI_MAIN_TREE:?main mode requires SHIROBAI_MAIN_TREE (a checkout of the shirobai main branch)}"
  export SHIROBAI_MAIN_TREE
fi

echo "corpus: $corpus  rounds: $rounds  modes: ${modes[*]}"
echo

mode_gemfile() {
  case "$1" in
    stock|removed) echo "$benchdir/Gemfile.realconfig.stock" ;;
    shirobai)      echo "$benchdir/Gemfile.realconfig.shirobai" ;;
    main)          echo "$benchdir/Gemfile.realconfig.main" ;;
  esac
}

# The removed mode excludes every cop shirobai implements. Resolve the list once
# from the shirobai bundle (the registry is the source of truth).
except_list=""
if modes_contains removed; then
  echo "=== resolving implemented cop list for removed mode ==="
  except_list="$(BUNDLE_GEMFILE="$benchdir/Gemfile.realconfig.shirobai" \
    bundle exec ruby "$benchdir/implemented_cops.rb")"
  [[ -n "$except_list" ]] || { echo "error: implemented_cops.rb returned an empty list" >&2; exit 1; }
  echo "removed mode excludes $(echo "$except_list" | tr ',' '\n' | wc -l) cops"
  echo
fi

run_rubocop() {
  local mode="$1"; shift
  local gemfile extra_args
  gemfile="$(mode_gemfile "$mode")"
  case "$mode" in
    stock)    extra_args="" ;;
    removed)  extra_args="--except $except_list" ;;
    shirobai) extra_args="--require shirobai" ;;
    main)     extra_args="--require shirobai" ;;
  esac
  # Run from inside the corpus, like a user of that project would.
  # Relative Exclude patterns in default configs (rubocop core and
  # plugins, e.g. rubocop-rails's bin/*) anchor to the working
  # directory, so running from outside lints files users never lint.
  (cd "$corpus" && BUNDLE_GEMFILE="$gemfile" bundle exec rubocop \
    $extra_args --cache false --no-server "$@")
}

# provenance: record the resolved gem versions each mode uses, so a run can be
# reproduced and a version-driven change (e.g. a rubocop-ast parse-path shift)
# is visible in the log and the job summary.
provenance_lines=""
echo "=== provenance ==="
for mode in "${modes[@]}"; do
  line="$(BENCH_MODE="$mode" BUNDLE_GEMFILE="$(mode_gemfile "$mode")" bundle exec ruby -e '
    require "bundler"
    versions = Bundler.load.specs.each_with_object({}) { |s, h| h[s.name] = s.version.to_s }
    want = %w[rubocop rubocop-ast parser prism rubocop-performance]
    parts = want.filter_map { |n| "#{n}=#{versions[n]}" if versions[n] }
    puts "provenance: mode=#{ENV["BENCH_MODE"]} #{parts.join(" ")}"
  ')"
  echo "$line"
  provenance_lines+="$line"$'\n'
done
echo

offenses="" files=""
if [ "${VERIFY:-0}" = "1" ]; then
  workdir="$(mktemp -d)"
  echo "=== verify: shirobai-family offense sets must match stock (also warms the file cache) ==="
  for mode in "${modes[@]}"; do
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
  if modes_contains stock; then
    for mode in "${modes[@]}"; do
      case "$mode" in
        shirobai|main)
          echo "--- offense diff: stock vs $mode ---"
          ruby "$benchdir/offense_diff.rb" "$workdir/stock.json" "$workdir/$mode.json"
          ;;
      esac
    done
    offenses=$(ruby -rjson -e 'puts JSON.parse(File.read(ARGV[0]))["summary"]["offense_count"]' "$workdir/stock.json")
    files=$(ruby -rjson -e 'puts JSON.parse(File.read(ARGV[0]))["files"].size' "$workdir/stock.json")
  else
    echo "note: stock not in MODES; skipping offense-set verification"
  fi
  echo
fi

results=""

for i in $(seq "$rounds"); do
  for mode in "${modes[@]}"; do
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
  OFFENSES="$offenses" FILES="$files" SUMMARY_FILE="${SUMMARY_FILE:-}" \
  MODES="${modes[*]}" PROVENANCE="$provenance_lines" ruby -e '
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

modes = ENV["MODES"].to_s.split
med = {}
modes.each { |m| med[m] = median.call(vals[m]) unless vals[m].empty? }

modes.each do |m|
  next if vals[m].empty?
  printf "%-9s median: %.2fs  (min %.2f, max %.2f)\n", m, med[m], vals[m].min, vals[m].max
end

sm = med["stock"]; hm = med["shirobai"]; mm = med["main"]; rm = med["removed"]
branch_saving     = (sm && hm) ? sm - hm : nil
main_saving       = (sm && mm) ? sm - mm : nil
branch_minus_main = (branch_saving && main_saving) ? branch_saving - main_saving : nil
upper_bound       = (sm && rm) ? sm - rm : nil
achieved          = (branch_saving && upper_bound && upper_bound != 0) ? branch_saving / upper_bound * 100 : nil
pct = ->(x) { (sm && sm != 0) ? x / sm * 100 : 0.0 }

puts
printf "branch saving (stock - shirobai):     %.2fs (%.1f%%)\n", branch_saving, pct.call(branch_saving) if branch_saving
printf "main saving   (stock - main):         %.2fs (%.1f%%)\n", main_saving, pct.call(main_saving) if main_saving
printf "branch - main (+ = branch faster):    %.2fs (%.1f%%)\n", branch_minus_main, pct.call(branch_minus_main) if branch_minus_main
printf "upper bound   (stock - removed):      %.2fs (%.1f%%)\n", upper_bound, pct.call(upper_bound) if upper_bound
printf "achieved      (branch / upper bound): %.1f%%\n", achieved if achieved

summary = ENV["SUMMARY_FILE"].to_s
unless summary.empty?
  File.open(summary, "a") do |f|
    f.puts "### #{ENV["CORPUS_NAME"]}"
    f.puts
    unless ENV["OFFENSES"].to_s.empty?
      f.puts "Verified: stock and shirobai-family modes report the same " \
             "#{ENV["OFFENSES"]} offenses over #{ENV["FILES"]} files (corpus config)."
      f.puts
    end
    f.puts "| mode | median | min | max |"
    f.puts "|---|---|---|---|"
    modes.each do |m|
      next if vals[m].empty?
      f.puts format("| %s | %.2fs | %.2fs | %.2fs |", m, med[m], vals[m].min, vals[m].max)
    end
    f.puts
    f.puts "| metric | value |"
    f.puts "|---|---|"
    f.puts format("| branch saving (stock - shirobai) | **%.2fs (%.1f%%)** |", branch_saving, pct.call(branch_saving)) if branch_saving
    f.puts format("| main saving (stock - main) | %.2fs (%.1f%%) |", main_saving, pct.call(main_saving)) if main_saving
    f.puts format("| branch - main (+ = branch faster) | **%.2fs (%.1f%%)** |", branch_minus_main, pct.call(branch_minus_main)) if branch_minus_main
    f.puts format("| upper bound (stock - removed) | %.2fs (%.1f%%) |", upper_bound, pct.call(upper_bound)) if upper_bound
    f.puts format("| achieved (branch / upper bound) | %.1f%% |", achieved) if achieved
    f.puts
    prov = ENV["PROVENANCE"].to_s
    unless prov.strip.empty?
      f.puts "```"
      f.puts prov.strip
      f.puts "```"
      f.puts
    end
  end
end
'
