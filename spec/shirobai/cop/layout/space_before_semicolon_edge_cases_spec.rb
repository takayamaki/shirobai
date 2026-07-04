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
