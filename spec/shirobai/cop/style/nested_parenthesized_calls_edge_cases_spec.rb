# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/NestedParenthesizedCalls`.
#
# The vendor spec covers a single-arg nested call, a multi-arg one, a safe-nav
# inner call, the four "allowed_omission" rejections (no args, parenthesized,
# setter, allowed-method-with-one-arg, deeply nested via block) and a backslash
# newline. Real-machine stock probing turned up several quirks that the vendor
# spec does not exercise — pinned here as differential tests so a refactor (or
# a stricter corpus) cannot silently regress them.
#
# - `AllowedMethods` allowance gates on BOTH the inner call AND the parent
#   argument list having exactly one element: `expect(obj).to(eq 1)` passes,
#   but `expect(obj).to(eq 1, 2)` flags (the inner has 2 args) and so does
#   `foo(eq 1, bar)` (the parent's argument list has 2 elements).
# - Operator methods (`+`, `==`, `<=>`, ...) and aref `[]` are exempt — they
#   come through prism's `CallNode` with the operator as `name`, so the
#   `OPERATOR_METHODS` predicate must catch them.
# - Setter `name=` is exempt, but the assignment-method exception drops the
#   comparison-operator names (`==`, `===`, `!=`, `<=`, `>=`) so a comparison
#   inside a paren'd outer is not mistaken for a setter.
# - `obj.[] 1` is a CallNode whose name is `[]` — operator_method? true, so it
#   is exempt even though its paren is missing.
# - Deeply nested unparen calls (`a(b(c d))`) flag the INNERMOST unparen call;
#   the middle `b(c d)` is itself parenthesized and so is allowed by the
#   outer's `allowed_omission?`.
# - csend on the OUTER call (`a&.puts(foo bar)`) — `alias on_csend on_send`,
#   so safe-nav outer calls drive the cop just like ordinary sends.
# - A ternary as an argument is NOT a `:call` child (`if`/`unless` node) and so
#   never feeds the inner check (`each_child_node(:call)` filters by type).
# - A block argument (`block_taker { another_method 1 }`) is a `:block` node,
#   not a `:call` child — even though its body looks like a flagged shape,
#   the cop must NOT flag.
# - Outer-only patterns (`puts foo bar`) — the cop bails on the outer's
#   `parenthesized?` check, so unparen'd outers stay untouched.
RSpec.describe Shirobai::Cop::Style::NestedParenthesizedCalls do
  include EdgeCaseParity

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Style/NestedParenthesizedCalls" => {
            "AllowedMethods" => %w[
              be be_a be_an be_between be_falsey be_kind_of be_instance_of
              be_truthy be_within eq eql end_with include match raise_error
              respond_to start_with
            ]
          }
        },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::NestedParenthesizedCalls,
    Shirobai::Cop::Style::NestedParenthesizedCalls
  ]

  describe "AllowedMethods gating" do
    it "passes single-arg allowed inner with single-arg parent" do
      expect_lint_parity(*klasses, "expect(obj).to(eq 1)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "expect(obj).to(eq 1)\n", config)).to be_empty
    end

    it "flags an allowed inner when the inner has multiple arguments" do
      expect_lint_parity(*klasses, "expect(obj).to(eq 1, 2)\n", config)
    end

    it "flags an allowed inner when the PARENT has multiple arguments" do
      expect_lint_parity(*klasses, "foo(eq 1, bar)\n", config)
    end
  end

  describe "operator / aref / setter exemptions" do
    it "does NOT flag operator method calls" do
      ["foo(a + b)\n", "foo(a == b)\n", "foo(a <=> b)\n"].each do |src|
        expect_lint_parity(*klasses, src, config, expect_offenses: false)
        expect(lint_offenses(klasses.first, src, config)).to be_empty
      end
    end

    it "does NOT flag aref calls (`obj[1]`)" do
      expect_lint_parity(*klasses, "method(obj[1])\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "method(obj[1])\n", config)).to be_empty
    end

    it "does NOT flag explicit `[]` operator-method calls" do
      # `obj.[] 1` -> CallNode name=`[]`, exempt by operator_method?.
      expect_lint_parity(*klasses, "foo(obj.[] 1)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo(obj.[] 1)\n", config)).to be_empty
    end

    it "does NOT flag setter calls" do
      expect_lint_parity(
        *klasses,
        "expect(object1.attr = 1).to eq 1\n",
        config,
        expect_offenses: false
      )
      expect(
        lint_offenses(klasses.first, "expect(object1.attr = 1).to eq 1\n", config)
      ).to be_empty
    end
  end

  describe "non-call argument shapes" do
    it "does NOT flag block argument (block node, not call child)" do
      src = "method(block_taker { another_method 1 })\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "does NOT flag ternary argument (if node, not call child)" do
      src = "puts(cond ? a b : c)\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "does NOT touch a non-parenthesized outer call" do
      src = "puts compute something\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end
  end

  describe "csend (safe navigation) on the outer call" do
    it "flags a csend outer call like an ordinary send" do
      expect_lint_parity(*klasses, "a&.puts(compute foo)\n", config)
    end
  end

  describe "deeply nested unparenthesized calls" do
    it "flags only the innermost unparenthesized one" do
      # `a(b(c d))` -> a paren'd, b paren'd (allowed), c d unparen (flag).
      stock = expect_lint_parity(*klasses, "a(b(c d))\n", config)
      expect(stock.size).to eq(1)
    end
  end

  describe "autocorrect" do
    it "wraps the inner arguments in parens (whitespace replacement + close)" do
      expect_autocorrect_parity(*klasses, "puts(compute something)\n", config)
    end

    it "eats a backslash continuation in the leading whitespace" do
      expect_autocorrect_parity(*klasses, "puts(nex \\\n      5)\n", config)
    end

    it "rewrites a safe-nav inner call" do
      expect_autocorrect_parity(*klasses, "puts(receiver&.compute something)\n", config)
    end

    it "rewrites a multi-arg inner call" do
      expect_autocorrect_parity(*klasses, "puts(compute first, second)\n", config)
    end
  end
end
