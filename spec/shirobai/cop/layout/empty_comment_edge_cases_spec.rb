# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EmptyComment`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. CRLF line endings. Prism's comment `location` ends at the `\r`
#      (slice `"#\r"`); parser-gem's `comment.source_range` ends at the `#`
#      (slice `"#"`). Stock's offense range is the parser-gem range, so the
#      Rust side must snap the prism end back by one byte. Autocorrect drops
#      the entire `#\r\n` (3 bytes).
#   2. Trailing whitespace on the comment line: `comment.text.strip` removes
#      the trailing space/tab/CR, so `# \n` still pattern-matches `#\n`.
#   3. Block comments (`=begin/=end`) span two lines, so the column-and-line
#      chunking naturally breaks at them — a lonely `#` two lines after the
#      block comment forms a separate chunk and is flagged on its own.
#   4. Indented chunks at different columns split into separate chunks (column
#      mismatch); each indented chunk is independently pattern-matched.
#   5. EOF without a trailing newline: a lonely `#` at end-of-file still
#      reports the offense, and the autocorrect whole-line removal range
#      clamps at source length (no `\n` to consume).
#   6. Border in the middle of a chunk under `AllowBorderComment: false`:
#      the joined chunk includes the border `#####\n` segment, but the
#      `/\A(#+\n)+\z/` pattern matches it, so every comment in the chunk
#      (including the border itself) becomes its own offense.
#   7. Inline comment without leading whitespace (`def foo#`): stock's
#      `range_with_surrounding_space(newlines: false)` finds nothing to expand
#      so the autocorrect range stays at the single `#`.
#
# All cases are differential against the 1.87-pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::EmptyComment do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::EmptyComment
  shirobai_klass = Shirobai::Cop::Layout::EmptyComment

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def with_cop_config(base_config, hash)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyComment" => hash }, "(test)"),
      "(test)"
    )
  end

  it "snaps prism's CRLF-included comment range back to parser-gem's range" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "#\r\n", cfg)
  end

  it "treats `# \\n` (trailing space) as empty after strip" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "#  \n", cfg)
  end

  it "splits chunks across a `=begin/=end` block comment" do
    src = "#\n=begin\n=end\n#\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "splits chunks when consecutive lines start at different columns" do
    src = "  #\n  #\nx = 1\n   #\n   #\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "reports a lonely `#` at EOF without a trailing newline" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1\n#", cfg)
  end

  it "flags every comment in a chunk that includes a border under AllowBorderComment: false" do
    src = "#\n#####\n#\n"
    config = with_cop_config(cfg, "AllowBorderComment" => false, "AllowMarginComment" => true)
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, config)
  end

  it "leaves a `def foo#` without leading space alone (no surrounding space to consume)" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "def foo#\n  bar\nend\n", cfg)
  end

  it "drops the leading space when the inline comment is `def foo #`" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "def foo #\n  bar\nend\n", cfg)
  end

  it "flags both inline comments aligned at the same column" do
    src = "def foo     #\n  bar       #\nend\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "keeps a chunk wrapping a text line (margin) under default config" do
    expect_lint_parity(stock_klass, shirobai_klass, "#\n# desc\n#\nclass Foo\nend\n", cfg,
                       expect_offenses: false)
  end

  it "flags the margin lines but not the text under AllowMarginComment: false" do
    src = "#\n# desc\n#\nclass Foo\nend\n"
    config = with_cop_config(cfg, "AllowBorderComment" => true, "AllowMarginComment" => false)
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, config)
  end

  it "allows `##` as a border under AllowBorderComment: true (default)" do
    expect_lint_parity(stock_klass, shirobai_klass, "##\n", cfg, expect_offenses: false)
  end

  it "flags `##` when AllowBorderComment: false" do
    config = with_cop_config(cfg, "AllowBorderComment" => false, "AllowMarginComment" => true)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "##\n", config)
  end

  it "leaves a `# rubocop:disable` directive comment alone" do
    src = "# rubocop:disable Layout/LineLength\nclass Foo\nend\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end
end
