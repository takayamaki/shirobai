# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceInsideReferenceBrackets`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. Empty-bracket offenses fire BEFORE the multiline guard: `foo[\n]` is
#      flagged under both empty styles.
#   2. `multiline?` runs on stock's legacy node: `a[1] = \n 2` (one `[]=`
#      send) is skipped, `a[1] += \n 2` (the inner `[]` send stays on one
#      line) is flagged.
#   3. The `Index*Write` / masgn-target forms are all reference brackets.
#   4. A comment inside the brackets makes the node multiline: skipped.
#   5. Array literals, explicit `a.[](1)` calls and `%w[…]` are not
#      reference brackets; a multiline receiver silences the index.
#   6. A heredoc opener argument keeps the node single-line.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideReferenceBrackets do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceInsideReferenceBrackets
  shirobai_klass = Shirobai::Cop::Layout::SpaceInsideReferenceBrackets

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def sirb_config(style, empty: "no_space")
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideReferenceBrackets" => {
          "EnforcedStyle" => style, "EnforcedStyleForEmptyBrackets" => empty
        } }, "(test)"
      ),
      "(test)"
    )
  end

  it "flags multiline empty brackets under both empty styles" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo[\n]\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo[\n]\n", sirb_config("no_space", empty: "space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo[  ]\n", sirb_config("no_space", empty: "space"))
    expect_lint_parity(stock_klass, shirobai_klass,
                       "foo[ ]\n", sirb_config("no_space", empty: "space"),
                       expect_offenses: false)
  end

  it "follows the legacy node extent for the multiline guard" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "a[ 1 ] =\n  2\n", sirb_config("no_space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ] +=\n  2\n", sirb_config("no_space"))
  end

  it "checks all assignment forms" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ] = 2\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ] += 2\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ] ||= 2\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ] &&= 2\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ], b = 1, 2\n", sirb_config("no_space"))
  end

  it "skips brackets made multiline by a comment or receiver" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "a[ # c\n1]\n", sirb_config("no_space"), expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass,
                       "foo[ # c\n ]\n", sirb_config("no_space"), expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass,
                       "(a\n)[ 1 ]\n", sirb_config("no_space"), expect_offenses: false)
  end

  it "ignores non-reference brackets" do
    src = "x = [ 1 ]\ny = a.[](1)\nz = a&.[](1)\nw = %w[ a ]\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, sirb_config("no_space"),
                       expect_offenses: false)
  end

  it "keeps a heredoc-opener index single-line" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ <<~EOS ]\n  b\nEOS\n", sirb_config("no_space"))
  end

  it "handles nested and chained indexes with per-node corrections" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ b[ 1 ] ]\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[ 1 ][ 2 ]\n", sirb_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a[b[1]]\n", sirb_config("space"))
  end

  it "treats tabs as offending spaces under no_space" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "hash[\t:key\t]\n", sirb_config("no_space"))
    expect_lint_parity(stock_klass, shirobai_klass,
                       "hash[\t:key\t]\n", sirb_config("space"), expect_offenses: false)
  end
end
