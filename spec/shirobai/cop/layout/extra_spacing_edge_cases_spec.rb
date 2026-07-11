# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/ExtraSpacing`.
#
# This cop scans the parser-gem TOKEN stream as adjacent pairs and then applies
# two side inputs the vendor spec does not pin as tightly as the corpus needs:
#
# - the `AllowForAlignment` filter reads the whole token list (columns of
#   aligned `=` / comparison operators and comment columns) through the shared
#   `Aligner`; shirobai feeds it the pm_lex stream translated by `rules::tokens`;
# - the `ignored_ranges` filter (multi-line hash key->value spans left to
#   `Layout/HashAlignment`) is MEMOIZED on the cop instance by stock, so across
#   an autocorrect re-pass on a reused instance the byte offsets go stale. The
#   wrapper reproduces that instance memoization, so the `autocorrect_run`
#   helper (one cop instance across passes) exercises exactly that path;
# - `ForceEqualSignAlignment` replaces "remove the space" with a block-align
#   autocorrect whose `@corrected` dedup + multi-pass convergence is the cop's
#   trickiest arm.
#
# Every case is a differential against the 1.88-pinned stock cop, generated
# fresh per file (no instance reuse for lint mode; one reused instance for the
# autocorrect loop, matching the vendor-spec iteration semantics). The
# `expect_offenses: false` cases assert BOTH sides stay at zero, so the
# alignment / ignored-range "allowed" paths cannot silently false-positive.
RSpec.describe Shirobai::Cop::Layout::ExtraSpacing do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::ExtraSpacing,
    Shirobai::Cop::Layout::ExtraSpacing
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def cop_config(overrides)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/ExtraSpacing" => { "Enabled" => true }.merge(overrides) },
        "(edge)"
      ),
      "(edge)"
    )
  end

  it "flags a same-line extra gap and removes it" do
    src = "x =  1\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("x = 1\n")
  end

  it "allows extra spaces used to vertically align assignments" do
    # The `=` columns line up, so `aligned_with_something?` reads the token
    # columns and keeps the leading extra space silent under AllowForAlignment.
    src = "a   = 1\nbbb = 2\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "flags that alignment once AllowForAlignment is off" do
    src = "a   = 1\nbbb = 2\n"
    cfg = cop_config("AllowForAlignment" => false)
    expect_lint_parity(*klasses, src, cfg)
    expect(expect_autocorrect_parity(*klasses, src, cfg)).to eq("a = 1\nbbb = 2\n")
  end

  it "flags an extra space before a trailing comment by default" do
    src = "x = 1  # c\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("x = 1 # c\n")
  end

  it "allows an extra space before a trailing comment under AllowBeforeTrailingComments" do
    src = "x = 1  # c\n"
    cfg = cop_config("AllowBeforeTrailingComments" => true)
    expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, cfg)).to be_empty
  end

  it "allows the key->value gap of a multi-line hash pair (HashAlignment territory)" do
    # `ignored_ranges`: the `[key.end, value.begin)` span of a pair in a
    # multi-line hash is left to Layout/HashAlignment. The wrapper applies this
    # filter with the same instance memoization stock uses.
    src = "{\n  a =>  1,\n  bb => 2,\n}\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "still flags the pair gap of a single-line hash" do
    # `pair.parent.single_line?` is true, so the pair is NOT ignored.
    src = "{ a =>  1 }\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("{ a => 1 }\n")
  end

  it "aligns misaligned `=` blocks under ForceEqualSignAlignment (multi-pass to convergence)" do
    # ForceEqualSignAlignment flags an `=` not aligned with the block max and
    # aligns the whole block. The autocorrect runs on ONE reused cop instance, so
    # the memoized `@ignored_ranges` (empty here) plus the `@corrected` dedup and
    # the convergence loop are all exercised end to end.
    src = "a = 1\nbbb = 2\nc = 3\n"
    cfg = cop_config("ForceEqualSignAlignment" => true)
    expect_lint_parity(*klasses, src, cfg)
    expect(expect_autocorrect_parity(*klasses, src, cfg)).to eq("a   = 1\nbbb = 2\nc   = 3\n")
  end

  it "keeps a multi-line hash pair ignored while aligning `=` under ForceEqualSignAlignment" do
    # A non-empty `@ignored_ranges` set that must stay applied across the reused
    # autocorrect instance: the hash pair gap stays silent while the surrounding
    # assignments are aligned.
    src = "aa = 1\nb = {\n  x =>  1,\n  yy => 2,\n}\n"
    cfg = cop_config("ForceEqualSignAlignment" => true)
    expect_lint_parity(*klasses, src, cfg)
    result = expect_autocorrect_parity(*klasses, src, cfg)
    # Whatever stock does, shirobai must match byte for byte.
    expect(lint_offenses(klasses.first, result, cfg)).to eq(lint_offenses(klasses.last, result, cfg))
  end
end
