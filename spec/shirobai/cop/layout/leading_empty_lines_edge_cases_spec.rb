# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/LeadingEmptyLines`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. Files starting with `__END__` carry NO parser-gem tokens at all
#      (`tokens` is empty), so the cop stays silent even with leading blank
#      lines before the marker. The Rust side mirrors this because prism
#      gives back an empty AST and empty comment list past the cutoff.
#   2. Inline `#` comments (including shebangs and magic comments) count as
#      the FIRST token at their line; if they live on line 1 the cop stays
#      silent regardless of later blank lines.
#   3. `=begin/=end` block comments count as one token covering the whole
#      block; the offense range matches stock's `tCOMMENT` end position
#      (block comments include the trailing `\n`, inline `#` ones do not).
#   4. BOM bytes are preserved in `buffer.source` and counted as ONE
#      character by parser-gem; the wrapper's `SourceOffsets` does the
#      byte→char conversion so the offense column lines up with stock.
#   5. CRLF blank lines (`\r\n\r\n`) are normalized to `\n\n` in
#      `buffer.source`; the wrapper falls back to the standalone path so
#      offsets index into `buffer.source` rather than `raw_source`.
#   6. Leading whitespace (spaces / tabs) on the first "blank" line still
#      counts as blank for line-counting purposes — the offense fires when
#      the first token's line is `> 1` regardless of indentation.
#   7. A leading indented first token on line 1 is `Layout/InitialIndentation`'s
#      problem, NOT this cop's — this cop only sees that line 1 has a
#      token, so it stays silent.
#   8. Empty files and whitespace-only files have no tokens and stay
#      silent.
#
# All cases are differential against the 1.87-pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::LeadingEmptyLines do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::LeadingEmptyLines
  shirobai_klass = Shirobai::Cop::Layout::LeadingEmptyLines

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "stays silent on `__END__` at the very start" do
    expect_lint_parity(stock_klass, shirobai_klass, "__END__\nfoo\n", cfg, expect_offenses: false)
  end

  it "stays silent on blanks preceding `__END__`" do
    expect_lint_parity(stock_klass, shirobai_klass, "\n__END__\nfoo\n", cfg,
                       expect_offenses: false)
  end

  it "fires on a blank line before an inline comment" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\n# c\n", cfg)
  end

  it "fires on a blank line before a `=begin/=end` block comment" do
    src = "\n=begin\nbody\n=end\nclass A; end\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "stays silent when a shebang is on line 1" do
    src = "#!/usr/bin/env ruby\n\nclass Foo\nend\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "fires when a blank line precedes the shebang" do
    src = "\n#!/usr/bin/env ruby\nclass Foo\nend\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "stays silent when a magic comment is on line 1" do
    src = "# frozen_string_literal: true\n\nclass A; end\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "fires on a BOM-prefixed source with a leading blank line" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\u{FEFF}\nputs 1\n", cfg)
  end

  it "stays silent on a BOM-prefixed source with no leading blank line" do
    expect_lint_parity(stock_klass, shirobai_klass, "\u{FEFF}puts 1\n", cfg,
                       expect_offenses: false)
  end

  it "fires on CRLF blank lines preceding code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\r\n\r\nputs 1\r\n", cfg)
  end

  it "fires on a tab-only line followed by code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\t\nputs 1\n", cfg)
  end

  it "fires on a space-only line followed by code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, " \nputs 1\n", cfg)
  end

  it "stays silent on an indented first-line token (Layout/InitialIndentation's job)" do
    expect_lint_parity(stock_klass, shirobai_klass, "  class A; end\n", cfg,
                       expect_offenses: false)
  end

  it "stays silent on an empty file" do
    expect_lint_parity(stock_klass, shirobai_klass, "", cfg, expect_offenses: false)
  end

  it "stays silent on a whitespace-only file" do
    expect_lint_parity(stock_klass, shirobai_klass, "   \n", cfg, expect_offenses: false)
  end

  it "stays silent on a newline-only file" do
    expect_lint_parity(stock_klass, shirobai_klass, "\n", cfg, expect_offenses: false)
  end

  it "fires on three leading blank lines before a class" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\n\n\nclass A; end\n", cfg)
  end

  it "fires on a blank line preceding an indented class (combined with InitialIndentation)" do
    # `Layout/LeadingEmptyLines` only cares that the first token's line > 1.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\n  class A; end\n", cfg)
  end
end
