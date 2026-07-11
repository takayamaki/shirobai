# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/EmptyLiteral`.
#
# The detection and autocorrect run stock's own `on_send` verbatim on the
# parser AST (so the `block` / `numblock` exclusion asymmetry and the
# parenless-argument wrapping are stock's by construction). The ONLY
# shirobai-specific part is `frozen_strings?`, whose two token-touching
# leading-comment scans (`frozen_string_literals_enabled?` /
# `frozen_string_literals_disabled?`) are replaced by token-free Rust. These
# differential cases pin that the Rust leading scan matches stock through the
# comment-prefix shapes the vendor spec under-tests: a shebang before the
# magic comment, a blank line, and code after `__END__`.
RSpec.describe Shirobai::Cop::Style::EmptyLiteral do
  include EdgeCaseParity

  def el_config(overrides = {})
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/EmptyLiteral" => { "Enabled" => true } }.merge(overrides), "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::EmptyLiteral,
    Shirobai::Cop::Style::EmptyLiteral
  ]

  describe "Array / Hash detection (stock on the parser AST)" do
    it "flags the empty-literal initializers" do
      ["test = Array.new\n", "test = ::Hash.new()\n", "test = Array[]\n", "test = Hash([])\n"].each do |src|
        expect_autocorrect_parity(*klasses, src, el_config)
      end
    end

    it "does not flag block / numblock / sized forms" do
      ["test = Array.new(3)\n", "test = Hash.new { block }\n", "test = Hash.new { _1[_2] = [] }\n",
       "test = Array.new { 1 }\n", "Hash[3, 4]\n"].each do |src|
        expect_lint_parity(*klasses, src, el_config, expect_offenses: false)
        expect(lint_offenses(klasses.first, src, el_config)).to be_empty
      end
    end

    it "wraps a parenless Hash.new argument to super in parentheses" do
      expect_autocorrect_parity(*klasses, "def foo\n  super Hash.new, something\nend\n", el_config)
    end
  end

  describe "String.new and the token-free frozen scan" do
    fslc_off = { "Style/FrozenStringLiteralComment" => { "Enabled" => false } }
    fslc_on = { "Style/FrozenStringLiteralComment" => { "Enabled" => true } }

    it "flags String.new when frozen string literals are off" do
      expect_autocorrect_parity(*klasses, "x = String.new\n", el_config(fslc_off))
    end

    it "does not flag String.new under a frozen magic comment (even after a shebang)" do
      src = "#!/usr/bin/env ruby\n# frozen_string_literal: true\nx = String.new\n"
      expect_lint_parity(*klasses, src, el_config(fslc_off), expect_offenses: false)
      expect(lint_offenses(klasses.first, src, el_config(fslc_off))).to be_empty
    end

    it "flags String.new under `frozen_string_literal: false` with the FSLC cop enabled" do
      src = "# frozen_string_literal: false\nx = String.new\n"
      expect_autocorrect_parity(*klasses, src, el_config(fslc_on))
    end

    it "does not flag String.new with the FSLC cop enabled and no magic comment" do
      expect_lint_parity(*klasses, "x = String.new\n", el_config(fslc_on), expect_offenses: false)
      expect(lint_offenses(klasses.first, "x = String.new\n", el_config(fslc_on))).to be_empty
    end

    it "does not treat a frozen comment after __END__ as leading" do
      # The magic comment sits in the __END__ data section, so it does not make
      # the file frozen: String.new still offends.
      src = "x = String.new\n__END__\n# frozen_string_literal: true\n"
      expect_autocorrect_parity(*klasses, src, el_config(fslc_off))
    end
  end
end
