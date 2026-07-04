# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceInsideParens`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. `tLPAREN_ARG` — the space-separated first-argument paren of a
#      parenless call (`f (3)`, `raise (x)`, `yield (1)`, `super (1)`,
#      `defined? (x)`, `not (x)`) — is not `left_parens?`: the left-side
#      checks never fire, while the `)` side still does. `return` / `break` /
#      `next` / `!` parens lex a plain `tLPAREN` and are fully checked.
#   2. Empty parens: `no_space` needs the pair on one line (`f(\n)` passes),
#      while `space` / `compact` flag any inner text that is not exactly
#      `()` — across lines and continuations too.
#   3. A comment right after the `(` silences the pair.
#   4. `%w()`-style closers are `tSTRING_END`, not `tRPAREN`; char literals
#      (`?)`) and string/comment bytes are not paren tokens.
#   5. `compact` flags consecutive same-direction parens only when the gap is
#      EXACTLY one space (a tab or two spaces pass).
#   6. Interpolation code is scanned; heredoc openers are same-line tokens.
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceInsideParens do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceInsideParens
  shirobai_klass = Shirobai::Cop::Layout::SpaceInsideParens

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def style_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideParens" => { "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  it "skips the left side of an ARG paren but not its right side" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f ( 3 )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f ( )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "yield ( 3 )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "not ( x )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "x.f ( 3 )\n", style_config("no_space"))
    expect_lint_parity(stock_klass, shirobai_klass, "f ( 3 )\n", style_config("space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f (3)\n", style_config("space"))
  end

  it "treats return/break/next/bang parens as plain parens" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "return ( 3 )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "while x\nbreak ( 1 )\nend\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "! ( x )\n", style_config("no_space"))
  end

  it "handles empty parens per style" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f( )\n", style_config("no_space"))
    expect_lint_parity(stock_klass, shirobai_klass, "f(\n)\n", style_config("no_space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(\n)\n", style_config("space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(\\\n)\n", style_config("space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(\n)\n", style_config("compact"))
    expect_lint_parity(stock_klass, shirobai_klass, "f()\n", style_config("space"),
                       expect_offenses: false)
  end

  it "silences a pair behind a comment" do
    src = "f( # c\n3)\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, style_config("no_space"),
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, src, style_config("space"),
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "f( \n3)\n", style_config("no_space"),
                       expect_offenses: false)
  end

  it "ignores paren bytes that are not paren tokens" do
    src = "x = ?)\ny = \"( )\"\nz = %w( a )\nw = 1 # ( )\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, style_config("no_space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "x = ( ?) )\n", style_config("no_space"))
    # `%w()`'s closer is a tSTRING_END: only the gap before `)` counts.
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "f(%w() )\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass, "f(%w())\n", style_config("space"))
  end

  it "compact flags consecutive parens only on a single-space gap" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "g( f( x ) )\n", style_config("compact"))
    expect_lint_parity(stock_klass, shirobai_klass, "g( f( x )  )\n", style_config("compact"),
                       expect_offenses: false)
    expect_lint_parity(stock_klass, shirobai_klass, "g( f( x )\t)\n", style_config("compact"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "g( ( 3 + 5 ) * f )\n", style_config("compact"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "g(  ( 3 ) )\n", style_config("compact"))
    expect_lint_parity(stock_klass, shirobai_klass, "g( f( x )\n)\n", style_config("compact"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "g( f ( x ) )\n", style_config("compact"))
  end

  it "scans interpolation code and heredoc openers" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "x = \"\#{f( 3 )}\"\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "f(<<~EOS )\n  b\nEOS\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "f(<<~EOS)\n  b\nEOS\n", style_config("space"))
  end

  it "checks def, lambda, destructuring and pattern parens" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "def f( a ); end\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "->( a ) { }\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "foo { |( a, b )| }\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "case x\nin Foo( 1 )\n  y\nend\n", style_config("no_space"))
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "a.( 1 )\n", style_config("no_space"))
  end

  it "ignores the __END__ data segment" do
    src = "x = 1\n__END__\nf( 3 )\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, style_config("no_space"),
                       expect_offenses: false)
  end
end
