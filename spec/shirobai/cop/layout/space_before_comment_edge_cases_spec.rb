# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceBeforeComment`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. A comment glued to an unquoted heredoc opener (`<<~EOS# c`) is an
#      offense — the identifier ends right before the `#`.
#   2. `=begin`/`=end` blocks are a single comment starting at column 0:
#      never flagged, and a `#` on the `=end … # tail` line is part of that
#      one comment, not a second one.
#   3. A `#` inside a heredoc body or a string is not a comment.
#   4. Only exact adjacency is an offense: a tab gap is accepted.
#   5. A comment at EOF without a trailing newline still works.
#   6. A semicolon glued to a comment is an offense (plus the semicolon
#      cop's own offense — each corrects independently).
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeComment do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceBeforeComment
  shirobai_klass = Shirobai::Cop::Layout::SpaceBeforeComment

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "flags a comment glued to a heredoc opener" do
    src = "foo <<~EOS# c\n  b\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "never flags =begin/=end blocks" do
    src = "=begin\ndoc\n=end\nx = 1\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
    src = "=begin\ndoc\n=end # tail\nx = 1\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags code after an embedded doc normally" do
    src = "=begin\ndoc\n=end\nx = 1# c\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "does not mistake # inside strings or heredocs for a comment" do
    src = "x = \"a# not\"\nf(<<~EOS)\n  b# not\nEOS\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "accepts a tab before the comment" do
    src = "x = 1\t# c\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags a comment at EOF without a trailing newline" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1# c", cfg)
  end

  it "flags a comment glued to a semicolon" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1;# c\n", cfg)
  end

  it "flags a comment glued to a percent literal" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = %w{a b}# c\n", cfg)
  end
end
