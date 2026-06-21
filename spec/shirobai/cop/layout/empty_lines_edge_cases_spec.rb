# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/EmptyLines`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. Plain-string literals (`"a\n\n\nb"`) and heredocs both emit one
#      `tSTRING_CONTENT` per file line of body, so the blank lines inside
#      them are NOT a gap and stock stays silent. The Rust side reconstructs
#      this by filling every line in the string body's span (via prism
#      `content_loc` for plain strings, via the inner StringNode parts +
#      closing_loc for heredocs).
#   2. Percent arrays (`%w[a\n\n\nb]`, `%i[]`) and ordinary array/hash
#      literals (`[1,\n\n\n2]`, `{ a: 1,\n\n\n b: 2 }`) do NOT have per-line
#      tokens, so the gap inside DOES trigger an offense — same as stock.
#   3. Three or more consecutive blank lines produce one offense per line
#      with both `lines[L - 2]` and `lines[L - 1]` empty (e.g. four blank
#      lines yield two offenses, not one — vendor only covers the two-blank
#      case).
#   4. `\n\n\n` at the very top of a file (before any token) yields offenses:
#      the loop seeds `prev_line = 1` and walks the gap starting from L=2.
#   5. `__END__` cuts off both prism and parser, so blank lines preceding it
#      are still considered for offenses up to the cutoff, but anything after
#      it (the data segment) is silent.
#   6. A comment between two statements counts as a token line and breaks the
#      gap (`a\n\n# foo\nb` is clean).
#   7. Heredoc body with internal blanks AND a separate gap below the
#      terminator: the body stays silent, the gap below offends.
#
# All cases are differential against the 1.87-pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::EmptyLines do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::EmptyLines
  shirobai_klass = Shirobai::Cop::Layout::EmptyLines

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "stays silent on blank lines inside a plain double-quoted string" do
    expect_lint_parity(stock_klass, shirobai_klass, "x = \"a\n\n\nb\"\n", cfg,
                       expect_offenses: false)
  end

  it "stays silent on blank lines inside a single-quoted string" do
    expect_lint_parity(stock_klass, shirobai_klass, "x = 'a\n\n\nb'\n", cfg,
                       expect_offenses: false)
  end

  it "stays silent on blank lines inside a heredoc body" do
    src = "x = <<~T\nline 1\n\n\nline 2\nT\nputs x\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags a `%w[]` percent array with blanks between elements" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = %w[a\n\n\nb]\n", cfg)
  end

  it "flags an array literal with blanks between elements" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = [1,\n\n\n2]\n", cfg)
  end

  it "flags every L where both predecessor lines are empty (four blanks → two offenses)" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "a = 1\n\n\n\nb = 2\n", cfg)
  end

  it "flags blank lines that precede the first token" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "\n\n\nfoo\n", cfg)
  end

  it "stays silent below `__END__` (parser cut-off)" do
    src = "x = 1\n\n\n\n__END__\nfoo\n\n\nbar\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags blanks that wrap an `__END__` substring inside a string literal" do
    # `__END__` inside a string is NOT the parser cut-off; parsing continues.
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = \"__END__\"\n\n\ny\n", cfg)
  end

  it "treats a comment as a token line that breaks the gap" do
    expect_lint_parity(stock_klass, shirobai_klass, "a\n\n# foo\nb\n", cfg,
                       expect_offenses: false)
  end

  it "flags a gap inside a multi-line def body" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "def foo\n  bar\n\n\n  baz\nend\n", cfg)
  end

  it "handles heredoc body with internal blanks plus a separate gap below" do
    src = "x = <<~T\na\n\n\nb\nT\n\n\ny = 1\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "stays silent on a single trailing blank line at end of file" do
    expect_lint_parity(stock_klass, shirobai_klass, "x = 1\n\n\n\n", cfg,
                       expect_offenses: false)
  end

  it "stays silent when there are no `\\n\\n\\n` substrings (prefilter)" do
    expect_lint_parity(stock_klass, shirobai_klass, "a\n\nb\n", cfg, expect_offenses: false)
  end

  it "flags a gap between blocks of `=begin/=end` block-comment data" do
    # `=begin/=end` is itself a comment that spans multiple file lines (one
    # `tCOMMENT` token at the `=begin` line). Stock's chunking groups it as
    # one token, so a gap of blanks above and below it is treated normally.
    src = "x = 1\n\n\n=begin\nblock\n=end\n\n\ny = 2\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end
end
