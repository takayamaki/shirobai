# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceBeforeSemicolon`.
#
# Quirks the vendor spec does not cover that surfaced during stock probing:
#
#   1. `space_required_after_lcurly?`: a space between a block-opening `{`
#      and the semicolon is skipped under `Layout/SpaceInsideBlockBraces`
#      `space` (the default) and flagged under `no_space`. A lambda's
#      `tLAMBEG` and `BEGIN` / `END` braces count as left curlies; a
#      `"#{ …}"` `tSTRING_DBEG` does not.
#   2. A semicolon that is the first token of the file has no `each_cons`
#      partner: never flagged.
#   3. `; ;` flags both whitespace runs.
#   4. Semicolon bytes inside opaque literals are not semicolon tokens
#      (`$;` included).
#
# All cases are differential against the pinned vendor cop.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeSemicolon do
  include EdgeCaseParity

  stock_klass = RuboCop::Cop::Layout::SpaceBeforeSemicolon
  shirobai_klass = Shirobai::Cop::Layout::SpaceBeforeSemicolon

  let(:cfg) { RuboCop::ConfigLoader.default_configuration }

  def block_braces_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceInsideBlockBraces" => { "EnforcedStyle" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  it "skips the gap after a block { under the space style, flags it under no_space" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "loop { ; 1 }\n", block_braces_config("space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "loop { ; 1 }\n", block_braces_config("no_space"))
  end

  it "treats a lambda tLAMBEG as a left curly" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "-> { ; 1 }\n", block_braces_config("space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "-> { ; 1 }\n", block_braces_config("no_space"))
  end

  it "treats BEGIN / END braces as left curlies" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "BEGIN { ; 1 }\nEND { ; 2 }\n", block_braces_config("space"),
                       expect_offenses: false)
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "BEGIN { ; 1 }\n", block_braces_config("no_space"))
  end

  it "treats a bare super block { as a left curly" do
    expect_lint_parity(stock_klass, shirobai_klass,
                       "def m; super { ; 1 }; end\n", block_braces_config("space"),
                       expect_offenses: false)
  end

  it "does not treat a tSTRING_DBEG as a left curly" do
    expect_autocorrect_parity(stock_klass, shirobai_klass,
                              "\"\#{ ;1}\"\n", block_braces_config("space"))
  end

  it "never flags a first-token semicolon" do
    expect_lint_parity(stock_klass, shirobai_klass, " ;x\n", cfg, expect_offenses: false)
  end

  it "flags both gaps of `; ;`" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1 ; ;\n", cfg)
  end

  it "ignores semicolon bytes inside opaque literals" do
    src = "x = \"a ;b\"\ny = 'c ;d'\nz = :\" ;\"\nr = / ;/\ng = $;\nc = ?;\n" \
          "l = 1 # a ;b\nh = <<~EOS\n  a ;b\nEOS\n"
    expect_lint_parity(stock_klass, shirobai_klass, src, cfg, expect_offenses: false)
  end

  it "flags block-parameter local separators" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "foo { |a ; b| b }\n", cfg)
  end

  it "flags a spaced semicolon at EOF without a trailing newline" do
    expect_autocorrect_parity(stock_klass, shirobai_klass, "x = 1 ;", cfg)
  end
end

# 1.92.3 (rubocop#16260): `SpaceBeforePunctuation#space_required_after?` also
# keeps the space after a `left_parens?` token (`tLPAREN` / `tLPAREN2`) when the
# enabled `Layout/SpaceInsideParens` style is `space` or `compact`. The vendor
# spec only uses `( ; )`. stock does NOT skip a `tLPAREN_ARG` (`foo ( ; 1)`),
# and prism has no token stream, so the wrapper rebuilds the token type from
# bytes. Each snippet below was probed on the parser gem lexer; the cases are
# differentials over all four SpaceInsideParens settings.
RSpec.describe Shirobai::Cop::Layout::SpaceBeforeSemicolon do
  include EdgeCaseParity

  # The helpers parse with `TargetRuby::DEFAULT_VERSION` (2.7), where
  # `foo ( ; 1)` is a syntax error; parse as 3.4 so the tLPAREN_ARG cases run.
  before { stub_const("RuboCop::TargetRuby::DEFAULT_VERSION", 3.4) }

  klasses = [
    RuboCop::Cop::Layout::SpaceBeforeSemicolon,
    Shirobai::Cop::Layout::SpaceBeforeSemicolon
  ]

  def parens_config(parens)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceBeforeSemicolon" => { "Enabled" => true }, "Layout/SpaceInsideParens" => parens },
        "(edge)"
      ),
      "(edge)"
    )
  end

  # tLPAREN / tLPAREN2: the space is kept under space / compact.
  lparen = [
    "( ; 1)\n", "x = ( ; 1)\n", "return ( ; 1)\n", "->( ;a) { a }\n", "foo(( ; 1))\n",
    "[( ; 1)]\n", "foo a, ( ; 1)\n", "!( ; 1)\n", "a && ( ; 1)\n", "foo[( ; 1)]\n",
    "if ( ; 1) then end\n", "a.b = ( ; 1)\n", "a + ( ; 1)\n", "x and ( ; 1)\n",
    "x if ( ; 1)\n", "while x\n  break ( ; 1)\nend\n", "case x\nwhen ( ; 1)\nend\n",
    "begin ( ; 1) end\n", "foo ? ( ; 1) : 2\n", "foo -( ; 1)\n", "foo ! ( ; 1)\n",
    "def m = ( ; 1)\n", "foo (( ; 1))\n", "foo *( ; 1)\n", "foo a: ( ; 1)\n",
    "x = [1,\n( ; 2)]\n"
  ]
  # tLPAREN_ARG and non-paren tokens: still an offense.
  lparen_arg = [
    "foo ( ; 1)\n", "foo.bar ( ; 1)\n", "foo\t( ; 1)\n", "puts ( ; 1).to_s\n",
    "not ( ; 1)\n", "super ( ; 1)\n", "defined? ( ; 1)\n", "def m\n  yield ( ; 1)\nend\n",
    "a&.b ( ; 1)\n", "Foo::Bar ( ; 1)\n", "Foo::bar ( ; 1)\n", "foo! ( ; 1)\n",
    "foo? ( ; 1)\n", "x = 1; x ( ; 1)\n", "foo.if ( ; 1)\n", "foo. if ( ; 1)\n",
    "Foo::if ( ; 1)\n", "foo \\\n( ; 1)\n", "foo\\\n( ; 1)\n", "fooé ( ; 1)\n",
    "it ( ; 1)\n", "_1 ( ; 1)\n", "x = ?( ; 1\n", "x = %w( ; 1)\n"
  ]

  {
    "space" => { "Enabled" => true, "EnforcedStyle" => "space" },
    "compact" => { "Enabled" => true, "EnforcedStyle" => "compact" },
    "no_space" => { "Enabled" => true, "EnforcedStyle" => "no_space" },
    "disabled" => { "Enabled" => false, "EnforcedStyle" => "space" }
  }.each do |name, parens|
    context "when SpaceInsideParens is #{name}" do
      (lparen + lparen_arg).each do |src|
        it "matches stock on #{src.inspect}" do
          cfg = parens_config(parens)
          stock = expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
          skips = %w[space compact].include?(cfg.for_enabled_cop("Layout/SpaceInsideParens")["EnforcedStyle"])
          expect(stock.empty?).to eq(skips && lparen.include?(src)) unless src.include?("%w(")
          expect_autocorrect_parity(*klasses, src, cfg)
        end
      end
    end
  end

  it "keeps the space after a tLPAREN but not after a tLPAREN_ARG" do
    cfg = parens_config("Enabled" => true, "EnforcedStyle" => "space")
    expect(lint_offenses(klasses.first, "( ; 1)\n", cfg)).to be_empty
    expect(lint_offenses(klasses.first, "foo ( ; 1)\n", cfg)).not_to be_empty
    expect(lint_offenses(klasses.first, "x = %w( ; 1)\n", cfg)).to be_empty
  end
end
