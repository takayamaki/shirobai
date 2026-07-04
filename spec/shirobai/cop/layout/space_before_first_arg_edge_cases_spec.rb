# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceBeforeFirstArg`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. ANY single whitespace character passes (`foo\tx` is fine).
#   2. A block-pass counts as an argument (`foo  &b` is an offense).
#   3. `not  x` is an operator method (`!`): never flagged.
#   4. The argument must sit on the node's FIRST line (`x.\n  foo  1` is
#      silent), and a continuation before the argument silences it too.
#   5. Alignment scans the nearest non-blank line per direction; comment and
#      blank lines are transparent; a second pass filters by the argument
#      line's indentation.
#   6. `aligned_words?` uses char columns (multibyte arguments align).
#   7. A `:sym=` first argument can align with the first
#      assignment-or-comparison token on the candidate line — `=`s inside
#      strings are not tokens and `<=>` is not in the token set (a naive
#      scan matching `<=` would mis-suppress).
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeFirstArg do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceBeforeFirstArg
  shirobai_klass = Shirobai::Cop::Layout::SpaceBeforeFirstArg

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def alignment_config(allow)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceBeforeFirstArg" => { "AllowForAlignment" => allow } }, "(test)"
      ),
      "(test)"
    )
  end

  it "accepts any single whitespace character" do
    expect_lint_parity(stock_klass, shirobai_klass, "something\tx\n", cfg,
                       expect_offenses: false)
  end

  it "counts block-pass, splat and kwsplat as first arguments" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  &b\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  &:sym\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  *a\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  **h\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x&.foo  1\n", cfg)
  end

  it "skips operator methods, setters and super/yield" do
    src = "x +  1\nnot  x\nx.foo =  1\nsuper  1\nyield  1\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "requires the argument on the node's first line" do
    expect_lint_parity(stock_klass, shirobai_klass, "x.\n  foo  1\n", cfg,
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "foo \\\n  bar\n", cfg,
                       expect_offenses: false)
  end

  it "suppresses aligned arguments through comment and blank lines" do
    expect_lint_parity(stock_klass, shirobai_klass, "foo  1\nbar  2\n", cfg,
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "foo  1\n# c\nbar  2\n", cfg,
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "foo  1\n\nbar  2\n", cfg,
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "foo    1\nbarbar 2\n", cfg,
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  1\nbar  2\n",
                              alignment_config(false))
  end

  it "only the nearest candidate line decides" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "baz\nfoo  1\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "wwwww 0\nfoo  1\n", cfg)
  end

  it "retries with the indentation filter on the second pass" do
    expect_lint_parity(stock_klass, shirobai_klass, "foo  1\n  x.bar\nbaz  2\n", cfg,
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo  1\n  indented  2\n", cfg)
  end

  it "aligns multibyte arguments by char columns" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "foo  実引数\nbar  x\n", cfg, expect_offenses: false)
  end

  it "aligns :sym= arguments with assignment tokens only" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "define  :foo=\nxxxxxxxxxxx = 1\n", cfg, expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "define  :foo=\nxxxxxxxxx = 111\n", cfg)
    # `=`s inside a string are not tokens.
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "define  :foo=\nx = \"===========\"\n", cfg)
    # `<=>` is a tCMP: longest-match must skip it.
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "define  :foo=\nzzzzzzzzzzz<=>b = 9\n", cfg)
  end

  it "flags a glued argument with an empty range" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "something'hello'\n", cfg)
  end

  it "ignores candidate lines past __END__" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo  1\n__END__\nbar  2\n", cfg)
  end
end
