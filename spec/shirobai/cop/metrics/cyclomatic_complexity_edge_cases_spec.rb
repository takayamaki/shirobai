# frozen_string_literal: true

require "spec_helper"
require_relative "../../../support/complexity_csend_reset_fixtures"

# Edge-case regression guard: which assignments reset the repeated `&.`
# discount.
#
# Stock (`RepeatedCsendDiscount`) counts only the first `var&.foo` on a local
# variable, and forgets the variable again at every parser-gem `:lvasgn` node.
# parser-gem emits `:lvasgn` not only for `x = v`, but also as the target
# inside `x += v`, `x ||= v`, `x &&= v`, `a, x = v` (also nested and splat),
# `for x in v` and `rescue => x`.
# Pattern captures (`in [x]`, `in Integer => x`, `in {x:}`) are `:match_var`
# and `/(?<x>.)/ =~ s` is `:match_with_lvasgn`, so those do NOT reset.
#
# prism uses `LocalVariableWriteNode` only for the plain `x = v` form.
# The others are `LocalVariable{Operator,Or,And}WriteNode` or a
# `LocalVariableTargetNode`, and prism also uses `LocalVariableTargetNode`
# for the pattern and `=~` captures that must not reset.
# Before the fix shirobai reset only on `LocalVariableWriteNode`, so it
# discounted the later `&.` and scored lower than stock
# (Ruby stdlib `prism/translation/parser/lexer.rb#to_a`: 116 vs 119).
#
# Vendor spec coverage: the vendor spec only resets with a plain `x = v`.

RSpec.describe Shirobai::Cop::Metrics::CyclomaticComplexity do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::CyclomaticComplexity,
    Shirobai::Cop::Metrics::CyclomaticComplexity
  ]
  let(:tight_config) { ComplexityCsendResetFixtures.config("Metrics/CyclomaticComplexity") }

  describe "repeated `&.` discount" do
    ComplexityCsendResetFixtures::RESETS.each do |name, body|
      it "is reset by #{name}" do
        expect_lint_parity(*klasses, ComplexityCsendResetFixtures.source(body), tight_config)
      end
    end

    it "is reset by `rescue => x`" do
      expect_lint_parity(*klasses, ComplexityCsendResetFixtures::RESCUE, tight_config)
    end

    ComplexityCsendResetFixtures::KEEPS.each do |name, body|
      it "is not reset by #{name}" do
        expect_lint_parity(*klasses, ComplexityCsendResetFixtures.source(body), tight_config)
      end
    end
  end
end
