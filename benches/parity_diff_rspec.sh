#!/usr/bin/env bash
# Real-CLI dual-Gemfile parity diff for the shirobai-rspec plugin —
# the truth oracle for RSpec-cop drop-in compatibility.
#
#   stock    = rubocop + rubocop-rspec           (Gemfile.stock.rspec)
#   shirobai = + shirobai + shirobai-rspec       (Gemfile.with_shirobai.rspec)
#
# This oracle deliberately does NOT use `--force-default-config` (the
# performance oracle's form): that flag skips the lint_roller plugin config
# merge, so `RSpec/Language` stays empty, the department Include never
# resolves, and every RSpec cop is silent ON BOTH SIDES — a zero-diff that
# verifies nothing. Instead a uniform config is written into the corpus
# root that `inherit_from`s the pinned rubocop-rspec default.yml (Language
# and department Include/Exclude included, with the corpus root as the
# Include base), and both CLIs run FROM INSIDE the corpus with `--config`.
#
# Before the corpus diff, the oracle self-tests on a synthetic fixture that
# must fire the implemented R1 cops on the stock side — so a config-plumbing
# regression can never produce an empty "both sides silent" parity again.
#
# TargetRubyVersion is pinned to 3.1 (a uniform middle ground: numbered
# block params exist, `it` block params do not). shirobai always parses
# with prism Latest; the known TargetRubyVersion caveats from the README
# apply here the same way.
#
# Usage:
#   benches/parity_diff_rspec.sh <corpus-path> [out-prefix] [target-dir]
#
# Examples:
#   benches/parity_diff_rspec.sh .tmp/discourse
#   benches/parity_diff_rspec.sh .tmp/factory_bot /tmp/fb spec
#
# Build shirobai first: `bundle exec rake compile`.
set -euo pipefail

corpus="${1:?usage: parity_diff_rspec.sh <corpus-path> [out-prefix] [target-dir]}"
prefix="${2:-/tmp/parity_rspec}"
target="${3:-spec}"
root="$(cd "$(dirname "$0")/.." && pwd)"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }
[[ -f "$root/Gemfile.stock.rspec" && -f "$root/Gemfile.with_shirobai.rspec" ]] \
  || { echo "error: rspec oracle Gemfiles missing at $root" >&2; exit 1; }
corpus="$(cd "$corpus" && pwd)"
[[ -d "$corpus/$target" ]] \
  || { echo "error: $corpus has no '$target' directory to lint" >&2; exit 1; }

# Resolve the pinned rubocop-rspec default.yml from the stock bundle.
rspec_default="$(BUNDLE_GEMFILE="$root/Gemfile.stock.rspec" bundle exec ruby \
  -e 'puts File.join(Gem.loaded_specs["rubocop-rspec"].gem_dir, "config", "default.yml")')"
[[ -f "$rspec_default" ]] \
  || { echo "error: could not resolve rubocop-rspec default.yml" >&2; exit 1; }

write_uniform_config() {
  local dir=$1
  cat > "$dir/.shirobai_rspec_parity.yml" <<EOF
inherit_from:
  - $rspec_default
AllCops:
  DisabledByDefault: false
  TargetRubyVersion: 3.1
EOF
}

# Runs one side. cd into the directory so the department Include base and
# relative Excludes anchor exactly like a user running rubocop there.
run_side() {
  local gemfile=$1 extra_require=$2 dir=$3 tgt=$4 out=$5
  ( cd "$dir" && BUNDLE_GEMFILE="$root/$gemfile" bundle exec rubocop \
      --config .shirobai_rspec_parity.yml --plugin rubocop-rspec \
      $extra_require --enable-pending-cops \
      --cache false --no-server -f json \
      "$tgt" > "$out" 2>/dev/null ) || true # rubocop exits non-zero on offenses
}

diff_jsons() {
  local stock_json=$1 sh_json=$2 label=$3
  ruby -rjson -e '
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
    stock_json, sh_json, label = ARGV
    st_files, st_total, st_h, st_per = load(stock_json)
    sh_files, sh_total, sh_h, sh_per = load(sh_json)
    puts format("stock    files=%-6d offenses=%d", st_files, st_total)
    puts format("shirobai files=%-6d offenses=%d", sh_files, sh_total)
    puts format("total diff: %+d", sh_total - st_total)
    cops = (st_h.keys + sh_h.keys).uniq.sort
    diffs = cops.reject { |c| sh_h[c] == st_h[c] }
    exact = cops.select { |c| sh_h[c] == st_h[c] }
                .reject { |c| ((sh_per[c] || {}).keys - (st_per[c] || {}).keys).empty? }
    diffs |= exact
    if diffs.empty?
      puts "per-cop divergence (#{label}): NONE (full parity)"
    else
      puts "per-cop divergence (#{label}, #{diffs.size} cops):"
      diffs.each do |c|
        only_sh = ((sh_per[c] || {}).keys - (st_per[c] || {}).keys).size
        only_st = ((st_per[c] || {}).keys - (sh_per[c] || {}).keys).size
        printf "  %-50s shirobai=%-6d stock=%-6d  diff=%+d  (sh-only=%d, st-only=%d)\n",
               c, sh_h[c], st_h[c], sh_h[c] - st_h[c], only_sh, only_st
      end
      puts
      puts "Inspect #{stock_json} / #{sh_json} per offense."
      exit 1
    end
  ' "$stock_json" "$sh_json" "$label"
}

# --- Oracle self-test on a synthetic fixture -------------------------------
# The fixture violates every R1 cop. The stock side must report each of
# them, proving the config path actually enables RSpec cops; then the
# fixture itself must show zero diff.
selftest_dir="$(mktemp -d /tmp/shirobai_rspec_selftest.XXXXXX)"
trap 'rm -rf "$selftest_dir"' EXIT
mkdir -p "$selftest_dir/spec"
cat > "$selftest_dir/spec/oracle_selftest_spec.rb" <<'FIXTURE'
describe 'oracle self-test' do
  let(:userName) { 1 }
  let('badString') { 2 }
  let!(:unused_setup) { 3 }

  it 'repeats' do
    expect(1).to eq(1)
  end

  it 'repeats' do
    expect(2).to eq(2)
  end

  context 'with too many helpers' do
    let(:a) { 1 }
    let(:b) { 2 }
    let(:c) { 3 }
    let(:d) { 4 }
    let(:e) { 5 }
    let(:f) { 6 }

    it 'is duplicated' do
      expect(3).to eq(3)
    end

    it 'is also duplicated' do
      expect(3).to eq(3)
    end
  end
end
FIXTURE
write_uniform_config "$selftest_dir"

echo "=== oracle self-test (synthetic fixture) ==="
run_side Gemfile.stock.rspec "" "$selftest_dir" spec "${prefix}_selftest_stock.json"
run_side Gemfile.with_shirobai.rspec "--require shirobai-rspec" \
  "$selftest_dir" spec "${prefix}_selftest_shirobai.json"

ruby -rjson -e '
  cops = %w[
    RSpec/VariableName RSpec/VariableDefinition RSpec/RepeatedDescription
    RSpec/RepeatedExample RSpec/MultipleMemoizedHelpers RSpec/LetSetup
  ]
  d = JSON.parse(File.read(ARGV[0]))
  fired = d["files"].flat_map { |f| f["offenses"].map { |o| o["cop_name"] } }.uniq
  missing = cops - fired
  unless missing.empty?
    warn "oracle self-test FAILED: stock did not fire #{missing.join(", ")}"
    warn "the uniform config is not enabling RSpec cops - the oracle would be empty"
    exit 1
  end
  puts "self-test: all #{cops.size} R1 cops fired on the stock side"
' "${prefix}_selftest_stock.json"

diff_jsons "${prefix}_selftest_stock.json" "${prefix}_selftest_shirobai.json" fixture

# --- Corpus diff ------------------------------------------------------------
stock_json="${prefix}_stock.json"
sh_json="${prefix}_shirobai.json"
write_uniform_config "$corpus"
cleanup_corpus() { rm -f "$corpus/.shirobai_rspec_parity.yml"; rm -rf "$selftest_dir"; }
trap cleanup_corpus EXIT

echo
echo "=== stock real-CLI (rubocop-rspec) on $corpus/$target ==="
time run_side Gemfile.stock.rspec "" "$corpus" "$target" "$stock_json"

echo
echo "=== shirobai real-CLI (shirobai-rspec) on $corpus/$target ==="
time run_side Gemfile.with_shirobai.rspec "--require shirobai-rspec" \
  "$corpus" "$target" "$sh_json"

echo
echo "=== summary ==="
diff_jsons "$stock_json" "$sh_json" corpus
