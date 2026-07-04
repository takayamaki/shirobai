# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceAfterComma`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. "Space missing" means the next token starts EXACTLY one column after
#      the comma: a tab gap is not an offense.
#   2. A comment right after the comma is a token pair and IS an offense.
#   3. A comma directly before a heredoc opener is an offense.
#   4. A `\`-newline continuation right after the comma puts the next token
#      on another line: no offense.
#   5. `,;` in block-local declarations has no `kind`: no comma offense.
#   6. A `}` after the comma is a `tRCURLY`: flagged under the hash-brace
#      cop's `space` style, allowed under `no_space`.
#   7. Comma bytes inside opaque literal regions are not comma tokens.
#   8. Pattern-matching and index commas are real comma tokens.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceAfterComma do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceAfterComma
  shirobai_klass = Shirobai::Cop::Layout::SpaceAfterComma

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def hash_braces_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideHashLiteralBraces" => { "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  it "accepts a tab after the comma (only exact adjacency is an offense)" do
    expect_lint_parity(stock_klass, shirobai_klass, "f(a,\tb)\n", cfg, expect_offenses: false)
  end

  it "flags a comment glued to the comma" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(a,# c\n  b)\n", cfg)
  end

  it "flags a heredoc opener glued to the comma" do
    src = "f(x,<<~EOS)\n  body\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "skips a backslash continuation after the comma" do
    src = "f(a,\\\n  b)\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "skips a comma directly followed by a block-local semicolon" do
    src = "foo { |a,;b| b }\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "treats a following } by the hash-brace style" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "h = {foo: 1,}\n", hash_braces_config("space"))
    expect_lint_parity(stock_klass, shirobai_klass,
                       "h = {foo: 1,}\n", hash_braces_config("no_space"),
                       expect_offenses: false)
  end

  it "ignores comma bytes inside opaque literals" do
    src = "x = \"a,b\"\ny = 'c,d'\nz = :\",\"\nw = %w{a,b}\nr = /,/\n" \
          "g = $,\nc = ?,\nl = 1 # a,b\ns = %i{a,b}\nt = `ls a,b`\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "ignores comma bytes in heredoc bodies but flags heredoc interpolation code" do
    src = "f(<<~EOS, x)\n  a,body\nEOS\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
    src = "f(<<~EOS)\n  pre \#{g(1,2)} post\nEOS\n"
    expect_autocorrect_parity(stock_klass, shirobai_klass, src, cfg)
  end

  it "ignores percent-array comma delimiters" do
    src = "x = %w,a b,\ny = %i,c d,\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "ignores comma bytes in the __END__ data segment" do
    src = "x = 1\n__END__\na,b\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags pattern-matching, index, when, rescue and undef commas" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nin [1,2] then y\nend\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "a[1,2]\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nwhen 1,2 then y\nend\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "begin\nrescue A,B\nend\n", cfg)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "undef :a,:b\n", cfg)
  end

  it "accepts allowed next tokens: ) ] |" do
    src = "f(a,)\n[1,]\nfoo { |a,| }\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags a comma at the last line without a trailing newline" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(a,b)", cfg)
  end
end
