# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceAfterSemicolon`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. A newline right after the semicolon produces no `tNL` token (the
#      lexer suppresses it), so `x = 1;\n` is never flagged.
#   2. `;;` sequences are skipped; only a `;` followed by a real token is
#      flagged (`x = 1;;y` flags the second `;`).
#   3. Allowed next tokens: `)`, `tSTRING_DEND`; a block-closing `}` follows
#      `Layout/SpaceInsideBlockBraces` (flagged under `space`, allowed under
#      `no_space`).
#   4. Block-local separators (`|a;b|`, `|a,;b|`) are real semicolons.
#   5. A comment glued to the semicolon is an offense.
#   6. Semicolon bytes inside opaque literals are not semicolon tokens.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceAfterSemicolon do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceAfterSemicolon
  shirobai_klass = Shirobai::Cop::Layout::SpaceAfterSemicolon

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def block_braces_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideBlockBraces" => { "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  it "never flags a semicolon at end of line" do
    src = "x = 1;\ny = 2\nz = 3;\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "skips semicolon sequences and flags the last of them" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1;;y = 2\n", cfg)
    expect_lint_parity(stock_klass, shirobai_klass, "x = 1;;\n", cfg, expect_offenses: false)
  end

  it "accepts ) and tSTRING_DEND as next tokens" do
    src = "(a;)\nx = \"\#{a;}\"\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "treats a closing block } by the block-brace style" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo {a;}\n", block_braces_config("space"))
    expect_lint_parity(stock_klass, shirobai_klass,
                       "foo {a;}\n", block_braces_config("no_space"),
                       expect_offenses: false)
  end

  it "flags block-local separators" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo { |a;b| b }\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo { |a,;b| b }\n", cfg)
  end

  it "flags a comment glued to the semicolon" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1;# c\n", cfg)
  end

  it "ignores semicolon bytes inside opaque literals and __END__ data" do
    src = "x = \"a;b\"\ny = 'c;d'\nz = :\";\"\nr = /;/\ng = $;\nc = ?;\n" \
          "l = 1 # a;b\n__END__\nx ;y\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags semicolons inside interpolation code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = \"\#{a;b}\"\n", cfg)
    src = "f(<<~EOS)\n  \#{a;b}\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "never flags a semicolon at EOF without a trailing newline" do
    expect_lint_parity(stock_klass, shirobai_klass, "x = 1;", cfg, expect_offenses: false)
  end
end
