# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceBeforeComma`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. Comma bytes inside opaque literal regions are not comma tokens:
#      strings, quoted symbols, `%w` words, regexps, comments, `$,`, `?,`,
#      heredoc bodies and the `__END__` data segment.
#   2. A heredoc opener followed by ` ,` on the same line IS a gap between
#      the `tSTRING_BEG` and the comma (heredoc content tokens sort after).
#   3. A `\`-newline continuation before a line-leading comma puts the
#      previous token on another line, so nothing is flagged.
#   4. Tab and multi-space gaps: the offense range covers the whole run.
#   5. Commas inside interpolation code (plain strings and heredocs alike)
#      are real comma tokens.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeComma do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceBeforeComma
  shirobai_klass = Shirobai::Cop::Layout::SpaceBeforeComma

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  it "ignores comma bytes inside opaque literals" do
    src = "x = \"a ,b\"\ny = 'c ,d'\nz = :\" ,\"\nw = %w{a ,b}\nr = / ,/\n" \
          "g = $,\nc = ?,\nl = 1 # a ,b\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "ignores comma bytes in heredoc bodies and quoted terminators" do
    src = "f(<<~EOS, x)\n  a ,body\nEOS\nf(<<'E,S', y)\n  b ,c\nE,S\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "ignores comma bytes in the __END__ data segment" do
    src = "x = 1\n__END__\na ,b\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags the gap between a heredoc opener and a same-line comma" do
    src = "foo(<<~EOS , x)\n  body\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "skips a comma led by a backslash continuation" do
    src = "f(a \\\n, b)\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "covers tab and multi-space gaps in one range" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(a\t,b)\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(a  ,b)\n", cfg)
  end

  it "flags commas inside interpolation code" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = \"\#{f(1 , 2)}\"\n", cfg)
    src = "f(<<~EOS)\n  pre \#{g(1 , 2)} post\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "flags a spaced comma in a multiple assignment left-hand side" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "a , b = 1, 2\n", cfg)
  end
end
