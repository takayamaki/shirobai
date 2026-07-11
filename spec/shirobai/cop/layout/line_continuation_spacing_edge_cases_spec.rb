# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/LineContinuationSpacing`.
#
# The Rust side supplies only stock's `last_line` (the line-scan bound stock
# reads from `processed_source.tokens.last.line`). The wrapper runs stock's own
# `on_new_investigation` body — the `\`-guard, the per-line
# `find_offensive_spacing` regex, `ignore_range?` over `ignored_literal_ranges`
# (`processed_source.ast`) + `comment_ranges` (`processed_source.comments`), and
# the autocorrect — so detection and the corrected bytes are stock's code by
# construction.
#
# The important shirobai-specific property to pin: shirobai's `last_line` shares
# `Layout/EndOfLine`'s definition and is HIGHER than stock's when the file's last
# statement ends with a heredoc (prism node end vs parser-gem's opener-line
# tNL). For `Layout/EndOfLine` that is a documented limitation, but here it is
# HARMLESS: the only extra lines scanned are heredoc body/terminator lines,
# whose backslashes sit inside `ignored_literal_ranges` (heredoc body) and are
# filtered by `ignore_range?`. These differential cases prove stock and shirobai
# still agree there.
RSpec.describe Shirobai::Cop::Layout::LineContinuationSpacing do
  include EdgeCaseParity

  def lcs_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/LineContinuationSpacing" => { "Enabled" => true, "EnforcedStyle" => style } },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Layout::LineContinuationSpacing,
    Shirobai::Cop::Layout::LineContinuationSpacing
  ]

  describe "space style (default)" do
    it "flags and corrects too many spaces before the backslash" do
      expect_autocorrect_parity(*klasses, "'a'  \\\n'b'\n", lcs_config("space"))
    end

    it "flags and corrects zero spaces before the backslash" do
      expect_autocorrect_parity(*klasses, "'a'\\\n'b'\n", lcs_config("space"))
    end
  end

  describe "no_space style" do
    it "flags and corrects a single space before the backslash" do
      expect_autocorrect_parity(*klasses, "'a' \\\n'b'\n", lcs_config("no_space"))
    end
  end

  describe "the last_line bound with a heredoc-tail file (shirobai scans more, but ignores it)" do
    # The last statement is a heredoc, so shirobai's `last_line` is the
    # terminator line (higher than stock's opener-line bound). The `  \` on the
    # heredoc body line is inside `ignored_literal_ranges`, so both agree: no
    # offense.
    it "does not flag a backslash inside a trailing heredoc body (space style)" do
      source = "x = <<~'E'\n  hi  \\\n  E\n"
      expect_lint_parity(*klasses, source, lcs_config("space"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, lcs_config("space"))).to be_empty
    end

    it "does not flag a backslash inside a trailing heredoc body (no_space style)" do
      source = "x = <<~'E'\n  hi \\\n  E\n"
      expect_lint_parity(*klasses, source, lcs_config("no_space"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, lcs_config("no_space"))).to be_empty
    end

    it "still flags an offensive continuation on the opener line before the heredoc" do
      # Self-test: a real offense on the code line, corrected identically.
      source = "'a'  \\\n'b'\nx = <<~'E'\n  hi\n  E\n"
      expect_autocorrect_parity(*klasses, source, lcs_config("space"))
    end
  end

  describe "ignored regions (differential)" do
    it "ignores a backslash inside a comment" do
      source = "x = 1 # foo  \\\ny = 2\n"
      expect_lint_parity(*klasses, source, lcs_config("space"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, lcs_config("space"))).to be_empty
    end
  end
end
