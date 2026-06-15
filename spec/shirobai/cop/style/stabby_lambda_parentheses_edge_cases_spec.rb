# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/StabbyLambdaParentheses`.
#
# The vendor spec covers only the three "common" no-flag cases (`lambda(&:nil?)`,
# `-> { true }`, `o.lambda`) plus one bad/good pair per style. Real-machine
# stock probing turned up a handful of quirks that the vendor spec does not
# exercise — pinned here as differential tests so a refactor cannot silently
# regress them.
#
# - `-> () { true }` parses (in prism) as a `LambdaNode` with a
#   `BlockParametersNode` whose inner `parameters` is nil. Stock's
#   `block_node.arguments?` is `false` for this (empty `(args)`), so neither
#   style flags it. The vendor spec does NOT cover this and a naive
#   "has opening_loc → flag under require_no_parentheses" check would
#   misfire.
# - `lambda { |a| a }` is a prism `CallNode` + `BlockNode`, not a
#   `LambdaNode`. Stock's `node.lambda_literal?` returns false for it, so it
#   stays clean under EITHER style — and stock's plain `lambda { ... }` is
#   handled by `Style/Lambda`, not this cop. A wrapper that walked
#   `BlockNode` too would over-flag.
# - Nested lambdas each get inspected independently — an outer good +
#   inner bad must yield exactly one offense (the inner).
# - The `arguments?` check is "args has at least one child", not "args.loc
#   has begin/end". `->(a) { a }` is fine under `require_parentheses` (the
#   stock + prism agreement on what counts as "has args").
RSpec.describe Shirobai::Cop::Style::StabbyLambdaParentheses do
  include EdgeCaseParity

  let(:require_parens_config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/StabbyLambdaParentheses" => {
            "EnforcedStyle" => "require_parentheses",
            "SupportedStyles" => %w[require_parentheses require_no_parentheses]
          }
        }, "(test)"
      ),
      "(test)"
    )
  end

  let(:require_no_parens_config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/StabbyLambdaParentheses" => {
            "EnforcedStyle" => "require_no_parentheses",
            "SupportedStyles" => %w[require_parentheses require_no_parentheses]
          }
        }, "(test)"
      ),
      "(test)"
    )
  end

  let(:klasses) do
    [
      RuboCop::Cop::Style::StabbyLambdaParentheses,
      Shirobai::Cop::Style::StabbyLambdaParentheses
    ]
  end

  describe "empty parens (`-> () { ... }`)" do
    # stock: `block_node.arguments?` is false for `s(:args)` with no children.
    # Neither style should flag this — even though prism's BlockParametersNode
    # does carry an opening_loc.
    it "does NOT flag under require_parentheses (no args at all)" do
      expect_lint_parity(
        *klasses, "-> () { true }\n", require_parens_config,
        expect_offenses: false
      )
    end

    it "does NOT flag under require_no_parentheses (no args at all)" do
      expect_lint_parity(
        *klasses, "-> () { true }\n", require_no_parens_config,
        expect_offenses: false
      )
    end
  end

  describe "no parens, no args (`-> { ... }`)" do
    it "does NOT flag under either style" do
      expect_lint_parity(
        *klasses, "-> { true }\n", require_parens_config,
        expect_offenses: false
      )
      expect_lint_parity(
        *klasses, "-> { true }\n", require_no_parens_config,
        expect_offenses: false
      )
    end
  end

  describe "lambda method call form (`lambda { |a| a }`)" do
    # Not a stabby lambda — `node.lambda_literal?` is false in stock and
    # prism distinguishes `CallNode + BlockNode` from `LambdaNode`. Never
    # flag, in either style.
    it "does NOT flag under require_parentheses" do
      expect_lint_parity(
        *klasses, "lambda { |a| a }\n", require_parens_config,
        expect_offenses: false
      )
    end

    it "does NOT flag under require_no_parentheses" do
      expect_lint_parity(
        *klasses, "lambda { |a| a }\n", require_no_parens_config,
        expect_offenses: false
      )
    end
  end

  describe "nested lambdas" do
    it "inspects each lambda independently (outer good, inner bad)" do
      stock = expect_lint_parity(
        *klasses, "->(x) { ->y { x + y } }\n", require_parens_config
      )
      expect(stock.size).to eq(1)
    end

    it "inspects each lambda independently (outer bad, inner good)" do
      stock = expect_lint_parity(
        *klasses, "->x { ->(y) { x + y } }\n", require_parens_config
      )
      expect(stock.size).to eq(1)
    end
  end

  describe "autocorrect under require_parentheses" do
    it "wraps the bare args with parentheses" do
      expect_autocorrect_parity(
        *klasses, "->a,b,c { a + b + c }\n", require_parens_config
      )
    end

    it "wraps a single bare arg with parentheses" do
      expect_autocorrect_parity(
        *klasses, "->a { a }\n", require_parens_config
      )
    end
  end

  describe "autocorrect under require_no_parentheses" do
    it "removes both parentheses, leaving the bare args" do
      expect_autocorrect_parity(
        *klasses, "->(a,b,c) { a + b + c }\n", require_no_parens_config
      )
    end
  end
end
