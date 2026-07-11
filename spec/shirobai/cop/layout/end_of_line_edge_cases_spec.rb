# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EndOfLine`.
#
# The Rust side supplies only stock's `last_line` (the line-scan bound that
# stock reads from `processed_source.tokens.last.line` — the "toucher" cost).
# The wrapper runs stock's own `on_new_investigation` body with that value, so
# detection and the offense range are stock's code by construction. These
# differential cases pin that the Rust `last_line` matches stock exactly on the
# behaviours the vendor spec under-tests: the `__END__` / trailing-line bound,
# the single-offense stop, and the `crlf` no-final-newline skip.
#
# KNOWN LIMITATION (documented, not shipped-broken): when the file's LAST
# top-level statement ENDS with a heredoc, parser-gem puts the final `tNL` on
# the heredoc OPENER line, while prism's node end is the terminator line, so the
# Rust `last_line` is higher and scans the heredoc body. This can only diverge
# on a file with MIXED line endings where the sole bad EOL sits on a heredoc
# body below its opener — a shape that never occurs in the LF verification
# corpora (all five are LF-only, so `EndOfLine` reports nothing on them) and is
# not exercised by the vendor spec. See `docs/cop-status.md`.
RSpec.describe Shirobai::Cop::Layout::EndOfLine do
  include EdgeCaseParity

  def eol_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/EndOfLine" => { "Enabled" => true, "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Layout::EndOfLine,
    Shirobai::Cop::Layout::EndOfLine
  ]

  describe "the last_line bound (Rust supplies it without the token stream)" do
    it "does not flag a trailing comment line's CR (beyond the last statement)" do
      # `tokens.last` is the tNL of `x = 1` on line 1, so line 2 is not scanned.
      source = "x = 1\n# trailing\r\n"
      expect_lint_parity(*klasses, source, eol_config("lf"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, eol_config("lf"))).to be_empty
    end

    it "does not flag a trailing blank CR+LF line (beyond the last statement)" do
      source = "x = 1\n\r\n"
      expect_lint_parity(*klasses, source, eol_config("lf"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, eol_config("lf"))).to be_empty
    end

    it "does not flag CR+LF lines inside the __END__ data section" do
      source = "x = 1\n__END__\ndata\r\nmore\r\n"
      expect_lint_parity(*klasses, source, eol_config("lf"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, eol_config("lf"))).to be_empty
    end

    it "flags a CR on the last statement line itself (within the bound)" do
      # Self-test fixture: stock DOES fire here (line 2 = last statement).
      source = "x = 1\ny = 2\r\n"
      stock = expect_lint_parity(*klasses, source, eol_config("lf"))
      expect(stock.map { |o| o[2] }).to all(include("Carriage return character detected"))
    end
  end

  describe "single-offense stop" do
    it "reports only the first offending line even when many lines are CR+LF" do
      source = "a = 0\r\nb = 1\r\nc = 2\r\n"
      stock = expect_lint_parity(*klasses, source, eol_config("lf"))
      expect(stock.length).to eq(1)
    end
  end

  describe "crlf style, no final newline" do
    it "does not flag a missing CR on the last line without a trailing LF" do
      # `unimportant_missing_cr?`: the last line has no LF, so a missing CR does
      # not matter under `crlf`.
      source = "x = 0"
      expect_lint_parity(*klasses, source, eol_config("crlf"), expect_offenses: false)
      expect(lint_offenses(klasses.first, source, eol_config("crlf"))).to be_empty
    end

    it "flags a missing CR on an earlier line under crlf" do
      source = "x = 0\ny = 1\r\n"
      stock = expect_lint_parity(*klasses, source, eol_config("crlf"))
      expect(stock.map { |o| o[2] }).to all(include("Carriage return character missing"))
    end
  end
end
