# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceAroundOperators`.
#
# This cop is a hybrid: an AST walk finds the operators and decides the
# missing- / detected- / single-space arms, but the excess-space (single-space)
# arm then defers to the `AllowForAlignment` filter, which reads the parser-gem
# TOKEN stream (columns of aligned `=` / comparison operators, comment columns,
# right-operand columns, ...). shirobai runs that filter over the pm_lex token
# stream translated by `rules::tokens`, so a refactor of either the walk
# callbacks or the token translation could silently regress the alignment
# decision. The vendor spec covers the basic arms but does NOT pin the
# token-driven quirks below.
#
# Every case is a differential against the 1.88-pinned stock cop, generated
# fresh per file (no instance reuse). `expect_lint_parity` also asserts the
# fixture fired at least one stock offense, so a mistyped source cannot pass
# vacuously; the `expect_offenses: false` cases assert BOTH sides stay at zero
# (the AllowForAlignment "allowed" path must not false-positive — that is the
# whole point of routing the token stream through the alignment filter).
RSpec.describe Shirobai::Cop::Layout::SpaceAroundOperators do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::SpaceAroundOperators,
    Shirobai::Cop::Layout::SpaceAroundOperators
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def cop_config(overrides)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceAroundOperators" => { "Enabled" => true }.merge(overrides) },
        "(edge)"
      ),
      "(edge)"
    )
  end

  it "corrects a chain of missing-space binaries (multi-pass to convergence)" do
    # Several adjacent missing-space operators; the autocorrect loop must run to
    # a fixpoint (the `other_offense_in_same_range?` accumulation trap).
    src = "x=1+2*3\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("x = 1 + 2 * 3\n")
  end

  it "flags the detected space around `**` under the default no_space exponent style" do
    src = "a ** b\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("a**b\n")
  end

  it "flags missing space around `**` under the space exponent style" do
    src = "a**b\n"
    cfg = cop_config("EnforcedStyleForExponentOperator" => "space")
    expect_lint_parity(*klasses, src, cfg)
    expect(expect_autocorrect_parity(*klasses, src, cfg)).to eq("a ** b\n")
  end

  it "allows a trailing excess space whose right operand aligns with the next line" do
    # `a =  1` has two spaces AFTER `=`, so the right operand `1` lines up in the
    # same column as `2` below. `aligned_with_something?` reads those token
    # columns and keeps the operator silent under the default AllowForAlignment.
    src = "a =  1\nbb = 2\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "flags that same trailing excess once AllowForAlignment is off" do
    src = "a =  1\nbb = 2\n"
    cfg = cop_config("AllowForAlignment" => false)
    expect_lint_parity(*klasses, src, cfg)
    expect(expect_autocorrect_parity(*klasses, src, cfg)).to eq("a = 1\nbb = 2\n")
  end

  it "allows a trailing excess space that aligns the value with a same-line comment" do
    # `comment_at_line` alignment: the value column matches the comment column,
    # which stock's `comment_excludes` allows. Reads the comment token column.
    src = "a =   1 # c\nbb =  2 # c\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "allows a hash-rocket with excess trailing space when the keys form a column" do
    # Regression for the redmine `query.rb` divergence: stock passes the WHOLE
    # pair (not its value) as the `right_operand`, so `aligned_with_something?`
    # measures alignment from the KEY column. The three `:key =>` pairs share a
    # column, so the middle pair's two-space `=>` is aligned and stays silent —
    # even though the values are NOT column-aligned. Measuring from the value
    # range (the original port's bug) would false-positive here.
    src = "super(\n" \
          "  :sortable => a || false,\n" \
          "  :totalable =>  b.key?(:t) ? c : d,\n" \
          "  :inline => e ? false : true\n" \
          ")\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "accepts a leading-spaced operator followed by a line continuation" do
    # 1.92.2 (rubocop#16148) `missing_surrounding_space?`: a gap that starts
    # with a space and is followed right away by `\` + newline is not missing
    # space. `range_with_surrounding_space` runs with continuations: false, so
    # the gap stops before the backslash.
    src = "a = 1 +\\\n  2\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "still flags a continuation operator with no leading space" do
    # The acceptance needs leading space: `1+\` is still missing space, and
    # the correction inserts a space between `+` and the backslash.
    src = "a = 1+\\\n  2\n"
    expect_lint_parity(*klasses, src, config)
    expect_correction_edits_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config)).to eq("a = 1 + \\\n  2\n")
  end

  it "flags a leading-spaced operator on a CRLF continuation like stock" do
    src = "a = 1 +\\\r\n  2\r\nb = 3+\\\r\n  4\r\n"
    expect_lint_parity(*klasses, src, config)
    expect_correction_edits_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "accepts hash rockets aligned on non-adjacent lines of the same hash" do
    # 1.92.2 (rubocop#16123): a `=>` whose column matches another `=>` of the
    # same hash is aligned, even when the lines in between break the
    # adjacent-line check.
    src = "{\n" \
          "  :a      => 1,\n" \
          "  :longer_key_here => 2,\n" \
          "  :b      => 3\n" \
          "}\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "counts char columns, not bytes, for the hash rocket tally" do
    # `"あい"` is 4 chars but 8 bytes: the two rockets share a char column
    # only. A byte-column tally would flag the first pair.
    src = "foo(\n" \
          "  \"あい\"   => 1,\n" \
          "  :something_long_x => 2,\n" \
          "  :abcd  => 3\n" \
          ")\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "does not tally rockets across nested hashes" do
    src = "{\n" \
          "  :a  => 1,\n" \
          "  :bbbbbbbbbbbbbbbbbbbb => { :c => 2 },\n" \
          "  :x => 1,\n" \
          "  :d  => { :e => 3,\n" \
          "           :f  => 4 }\n" \
          "}\n"
    expect_lint_parity(*klasses, src, config)
    expect_correction_edits_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end

  it "still flags a lone excess hash rocket" do
    src = "{\n  :a  => 1,\n  :bbbb => 2\n}\n"
    expect_lint_parity(*klasses, src, config)
    expect_autocorrect_parity(*klasses, src, config)
  end
end
