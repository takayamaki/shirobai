# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/InitialIndentation`.
#
# The Rust side is only a speed GATE ("is the first non-comment token
# indented?"); the wrapper builds the offense from stock's own `first_token`
# / `space_before`, so the offense range and autocorrect bytes are stock's by
# construction. These differential cases pin that the gate fires (and does not
# fire) exactly where stock does across the first-token shapes the gate must
# get right:
#
# - a `#` line comment is skipped, but a `=begin`/`=end` block comment is the
#   first token itself (always column 0), so indented code after a block
#   comment is NEVER reached — no offense;
# - the first token can be an identifier / keyword / ivar / cvar / gvar /
#   constant / `::` / string / symbol / number / `->` / `%w[` / heredoc opener
#   / `(`; the gate must detect indentation regardless of the token shape;
# - a byte-order mark makes the first token's column non-zero with no space to
#   its left (no offense), while indentation after the BOM does offend;
# - blank lines before a column-0 token do not offend.
RSpec.describe Shirobai::Cop::Layout::InitialIndentation do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::InitialIndentation,
    Shirobai::Cop::Layout::InitialIndentation
  ]

  describe "indented first token of every shape (offense)" do
    {
      "keyword def" => "  def f\n  end\n",
      "identifier call" => "  puts 1\n",
      "ivar" => "  @x = 1\n",
      "cvar" => "  @@x = 1\n",
      "gvar" => "  $x = 1\n",
      "constant" => "  Foo.bar\n",
      "colon3 const" => "  ::Foo\n",
      "double-quoted string" => "  \"foo\"\n",
      "single-quoted string" => "  'foo'\n",
      "integer literal" => "  123\n",
      "symbol" => "  :sym\n",
      "stabby lambda" => "  ->(x) { x }\n",
      "percent word array" => "  %w[a b]\n",
      "heredoc opener" => "  <<~X\n  hi\n  X\n",
      "open paren" => "  (1 + 2)\n",
      "if keyword" => "  if x\n  end\n",
      "tab indent" => "\tputs 1\n"
    }.each do |shape, source|
      it "matches stock for an indented #{shape}" do
        expect_autocorrect_parity(*klasses, source, config)
      end
    end
  end

  describe "indented code after a skipped comment (offense)" do
    it "flags indented code after an unindented `#` comment" do
      expect_autocorrect_parity(*klasses, "# c\n  x = 1\n", config)
    end

    it "flags indented code after an indented `#` comment, keeping the comment" do
      expect_autocorrect_parity(*klasses, "   # comment\n   x = 1\n", config)
    end
  end

  describe "cases that must NOT offend" do
    it "does not flag a `=begin`/`=end` block comment followed by indented code" do
      source = "=begin\nhi\n=end\n  code\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does not flag a column-0 start after blank lines" do
      source = "\n\ndef f\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does not flag an unindented comment + unindented code" do
      source = "# comment\nx = 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "byte-order mark" do
    it "does not flag a token right after the BOM (column 0)" do
      source = "\u{feff}puts 1\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "flags indentation after the BOM" do
      expect_autocorrect_parity(*klasses, "\u{feff}  puts 1\n", config)
    end

    it "flags indented code after a BOM + comment" do
      expect_autocorrect_parity(*klasses, "\u{feff}# comment\n  puts 1\n", config)
    end
  end
end
