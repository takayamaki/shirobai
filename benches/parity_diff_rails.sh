#!/usr/bin/env bash
# Real-CLI dual-Gemfile parity diff for the shirobai-rails plugin —
# the truth oracle for Rails-cop drop-in compatibility.
#
#   stock    = rubocop + rubocop-rails            (Gemfile.stock.rails)
#   shirobai = + shirobai + shirobai-rails        (Gemfile.with_shirobai.rails)
#
# Like the rspec oracle this deliberately does NOT use
# `--force-default-config`: that flag skips the lint_roller plugin config
# merge, so rubocop-rails's default.yml (department Include/Exclude and the
# TargetRailsVersion metadata) never resolves and every Rails cop is silent
# ON BOTH SIDES — a zero-diff that verifies nothing. Instead a uniform
# config is written into the corpus root that `inherit_from`s the pinned
# rubocop-rails default.yml, and both CLIs run FROM INSIDE the corpus with
# `--config`.
#
# The Application* cops (Record / Mailer / Job) are gated on
# `requires_gem('railties', '>= 5.0')`, which RuboCop resolves from the
# TARGET directory's Gemfile.lock (searched upward from the config's base
# dir). Real Rails corpora carry railties in their lockfile, so the gate
# activates on both sides identically; the synthetic self-test dir gets a
# minimal Gemfile.lock written so all four cops fire there too.
#
# `DisabledByDefault: true` (deviation from the rspec oracle's
# `DisabledByDefault: false`) scopes the run to the Rails department: the
# inherited rubocop-rails default.yml enables every Rails cop, and
# DisabledByDefault keeps the CORE / other departments off. This matters
# because Rails cops share files with core cops (the rspec oracle escapes
# this by targeting `spec/`, where its department is naturally file-scoped),
# and folding in core-cop parity would make this oracle re-litigate what
# benches/parity_diff.sh already owns — noise, not signal. shirobai-rails
# only replaces the four Application* cops; the other ~134 rubocop-rails cops
# run as STOCK on both sides, so they are byte-identical by construction —
# only the four replaced cops are actually under test.
#
# TargetRubyVersion is pinned to 3.1. shirobai always parses with prism
# Latest; the known TargetRubyVersion caveats from the README apply here the
# same way.
#
# Usage:
#   benches/parity_diff_rails.sh <corpus-path> [out-prefix] [target-dir]
#
# Examples:
#   benches/parity_diff_rails.sh .tmp/mastodon
#   benches/parity_diff_rails.sh .tmp/redmine /tmp/rd app
#
# Build shirobai first: `bundle exec rake compile`.
set -euo pipefail

corpus="${1:?usage: parity_diff_rails.sh <corpus-path> [out-prefix] [target-dir]}"
prefix="${2:-/tmp/parity_rails}"
target="${3:-app}"
root="$(cd "$(dirname "$0")/.." && pwd)"

[[ -d "$corpus" ]] || { echo "error: $corpus does not exist" >&2; exit 1; }
[[ -f "$root/Gemfile.stock.rails" && -f "$root/Gemfile.with_shirobai.rails" ]] \
  || { echo "error: rails oracle Gemfiles missing at $root" >&2; exit 1; }
corpus="$(cd "$corpus" && pwd)"
[[ -d "$corpus/$target" ]] \
  || { echo "error: $corpus has no '$target' directory to lint" >&2; exit 1; }

# Resolve the pinned rubocop-rails default.yml from the stock bundle.
rails_default="$(BUNDLE_GEMFILE="$root/Gemfile.stock.rails" bundle exec ruby \
  -e 'puts File.join(Gem.loaded_specs["rubocop-rails"].gem_dir, "config", "default.yml")')"
[[ -f "$rails_default" ]] \
  || { echo "error: could not resolve rubocop-rails default.yml" >&2; exit 1; }

write_uniform_config() {
  local dir=$1
  cat > "$dir/.shirobai_rails_parity.yml" <<EOF
inherit_from:
  - $rails_default
AllCops:
  DisabledByDefault: true
  TargetRubyVersion: 3.1
EOF
}

# Runs one side. cd into the directory so the department Include base, the
# relative Excludes and the railties lockfile all anchor exactly like a user
# running rubocop there.
run_side() {
  local gemfile=$1 extra_require=$2 dir=$3 tgt=$4 out=$5
  ( cd "$dir" && BUNDLE_GEMFILE="$root/$gemfile" bundle exec rubocop \
      --config .shirobai_rails_parity.yml --plugin rubocop-rails \
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
    # Only the four replaced cops are actually under test; still, any
    # divergence at all (including a stock cop drifting) fails the oracle.
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
# The fixture violates every Application* cop. The stock side must report
# each of them, proving the config path actually enables Rails cops and the
# railties gate is satisfied; then the fixture itself must show zero diff.
selftest_dir="$(mktemp -d /tmp/shirobai_rails_selftest.XXXXXX)"
trap 'rm -rf "$selftest_dir"' EXIT
mkdir -p "$selftest_dir/app"
cat > "$selftest_dir/app/oracle_selftest.rb" <<'FIXTURE'
class SelfTestModel < ActiveRecord::Base
end

class SelfTestController < ActionController::Base
end

class SelfTestMailer < ActionMailer::Base
end

class SelfTestJob < ActiveJob::Base
end

Anon = Class.new(ActiveRecord::Base)

# Rails/UnknownEnv (unknown environment name).
if Rails.env.staaging?
  do_something
end

# Rails/DynamicFindBy (dynamic finder with a receiver).
User.find_by_name_and_email(name, email)
FIXTURE
# Minimal lockfile so `gem_versions_in_target` resolves railties and the
# TargetRailsVersion-gated cops (Record / Mailer / Job) activate here.
cat > "$selftest_dir/Gemfile.lock" <<'LOCK'
GEM
  specs:
    railties (7.0.8)

PLATFORMS
  ruby

DEPENDENCIES
  railties
LOCK
write_uniform_config "$selftest_dir"

echo "=== oracle self-test (synthetic fixture) ==="
run_side Gemfile.stock.rails "" "$selftest_dir" app "${prefix}_selftest_stock.json"
run_side Gemfile.with_shirobai.rails "--require shirobai-rails" \
  "$selftest_dir" app "${prefix}_selftest_shirobai.json"

ruby -rjson -e '
  cops = %w[
    Rails/ApplicationRecord Rails/ApplicationController
    Rails/ApplicationMailer Rails/ApplicationJob
    Rails/UnknownEnv Rails/DynamicFindBy
  ]
  d = JSON.parse(File.read(ARGV[0]))
  fired = d["files"].flat_map { |f| f["offenses"].map { |o| o["cop_name"] } }.uniq
  missing = cops - fired
  unless missing.empty?
    warn "oracle self-test FAILED: stock did not fire #{missing.join(", ")}"
    warn "the uniform config / railties gate is not enabling the Application* cops"
    exit 1
  end
  puts "self-test: all #{cops.size} Application* cops fired on the stock side"
' "${prefix}_selftest_stock.json"

diff_jsons "${prefix}_selftest_stock.json" "${prefix}_selftest_shirobai.json" fixture

# --- Corpus diff ------------------------------------------------------------
stock_json="${prefix}_stock.json"
sh_json="${prefix}_shirobai.json"
write_uniform_config "$corpus"
cleanup_corpus() { rm -f "$corpus/.shirobai_rails_parity.yml"; rm -rf "$selftest_dir"; }
trap cleanup_corpus EXIT

echo
echo "=== stock real-CLI (rubocop-rails) on $corpus/$target ==="
time run_side Gemfile.stock.rails "" "$corpus" "$target" "$stock_json"

echo
echo "=== shirobai real-CLI (shirobai-rails) on $corpus/$target ==="
time run_side Gemfile.with_shirobai.rails "--require shirobai-rails" \
  "$corpus" "$target" "$sh_json"

echo
echo "=== summary ==="
diff_jsons "$stock_json" "$sh_json" corpus
