# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/ParenthesesAsGroupedExpression`.
#
# The vendor spec covers the canonical detection / autocorrect path and the
# main exclusions, but a number of structural quirks were uncovered during
# implementation by probing stock rubocop directly. Those quirks are pinned
# here because corpus parity is disposable (clean corpora produce zero
# offenses for this cop, so a divergence here would not surface) and because
# the AST shape the cop relies on differs subtly between parser-gem and prism
# in a few corner cases:
#
# - **The `ParenthesesNode` gate**: in prism only a `ParenthesesNode` first
#   argument starts with a `(` AND has a `loc.begin` equal to `(`. `CallNode`
#   chain receivers, `YieldNode`, `SuperNode`, `DefinedNode` all have
#   `loc.begin == (` for the parenthesised forms but their `source` does not
#   start with `(` (they start with the head token), so stock's
#   `source.start_with?('(')` guard already rejects them. The wrapper relies
#   on this and these cases must stay zero. The vendor spec covers `yield` /
#   `super` / `defined?` but not raw `CallNode` chain receivers.
# - **Operator method / setter method on the outer call**: an outer call whose
#   method name is an operator (`+`, `%`, …) or a setter (`x.b = …`) is
#   excluded by stock's `valid_context?`. The vendor spec covers `%` and `b=`
#   but not the full set; we pin a couple of representative operators here so
#   a refactor of `is_operator_method` would not silently regress.
# - **`(a)..(b)` first argument (compound range)**: stock excludes this via
#   `compound_range?` which checks `first_arg.range_type? && parenthesized_call?`.
#   In parser-gem the first arg is an `irange` whose `parenthesized_call?` is
#   false, so the FIRST `valid_context?` guard already excludes it. In prism
#   the first arg is a `RangeNode`, not a `ParenthesesNode`, so the
#   `ParenthesesNode` gate excludes it. Either way must stay zero.
# - **Simple range `(1..10)` first argument**: this DOES flag (the parens wrap
#   a single range expression, so the first arg is a `ParenthesesNode`). The
#   vendor spec covers this but only with `rand`; we pin a predicate-method
#   variant to guard the `?`-in-selector boundary on the autocorrect range.
# - **Nested context (call inside an `if` body / `begin`)**: the visitor must
#   recurse into method bodies and conditionals; pinned because a future
#   walk-pruning refactor could silently regress.
# - **Multiple spaces between selector and `(`**: the autocorrect must remove
#   the whole run; the vendor spec only covers single-space inputs.
# - **`if/case` first argument**: an `IfNode` first arg is not a
#   `ParenthesesNode` and is structurally rejected. The vendor spec covers
#   ternary but not `if expression`.
RSpec.describe Shirobai::Cop::Lint::ParenthesesAsGroupedExpression do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::ParenthesesAsGroupedExpression,
    Shirobai::Cop::Lint::ParenthesesAsGroupedExpression
  ]

  describe "first-argument shape gating" do
    it "does NOT flag when the first argument starts with `(` but is a CallNode chain" do
      # `(x).foo` is a `CallNode` whose receiver is a `ParenthesesNode`; the
      # first argument's `source.start_with?('(')` is true but stock's
      # `parenthesized_call?` is false (the `CallNode` source starts with
      # `(` only because the receiver does, the call itself has no opening
      # paren). Both stock and shirobai must stay zero.
      expect_lint_parity(*klasses, "func (x).chain\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "func (x).chain\n", config)).to be_empty
    end

    it "does NOT flag when the first argument is a `yield` with `(`" do
      expect_lint_parity(*klasses, "a yield(c)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a yield(c)\n", config)).to be_empty
    end

    it "does NOT flag when the first argument is a `super` with `(`" do
      expect_lint_parity(*klasses, "a super(c)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a super(c)\n", config)).to be_empty
    end

    it "does NOT flag when the first argument is a `defined?` with `(`" do
      expect_lint_parity(*klasses, "a defined?(c)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a defined?(c)\n", config)).to be_empty
    end

    it "does NOT flag when the first argument is an unparenthesised range" do
      # `rand (a - b)..(c - d)` — first_arg is a RangeNode whose ends are
      # parenthesised, but the first_arg itself is not a ParenthesesNode (its
      # `loc.begin` is nil, parser-gem `parenthesized_call?` is false).
      expect_lint_parity(*klasses, "rand (a - b)..(c - d)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "rand (a - b)..(c - d)\n", config)).to be_empty
    end

    it "does NOT flag when the first argument is an `if` expression" do
      # `if … end` first arg is an IfNode, not ParenthesesNode. The vendor
      # spec covers ternary IfNode but not `if expr`.
      source = "foo if a then b else c end\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "outer method name gating" do
    it "does NOT flag when the outer method is an operator method" do
      # `a + (b)` — outer = `+`, in OPERATOR_METHODS, excluded by
      # `node.operator_method?`. Pinned alongside the vendor `%` case so a
      # change to the operator table catches multiple representatives.
      expect_lint_parity(*klasses, "a + (b)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a + (b)\n", config)).to be_empty
    end

    it "does NOT flag when the outer method is `<=>`" do
      # `<=>` is a multi-char operator method; guards that the matcher
      # accepts non-single-byte operator names.
      expect_lint_parity(*klasses, "a <=> (b)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a <=> (b)\n", config)).to be_empty
    end

    it "does NOT flag when the outer method is a custom setter (`foo=`)" do
      # `obj.bar = (c)` — outer method is `bar=` (setter), excluded by
      # `node.setter_method?`. The vendor `a.b = (c == d)` is the canonical
      # case; this variant guards a setter name that is not single-char.
      expect_lint_parity(*klasses, "obj.bar = (c)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "obj.bar = (c)\n", config)).to be_empty
    end
  end

  describe "selector-end boundary" do
    it "flags a predicate method with `?` followed by a `(`" do
      # Pins that `message_loc().end_offset()` covers the `?` byte — so the
      # offense / autocorrect range starts AFTER the `?`, not before.
      expect_lint_parity(*klasses, "is? (x)\n", config)
    end

    it "removes the whole run of multiple spaces on autocorrect" do
      # The vendor spec only covers single-space inputs; pin that the range
      # is `[message_loc.end_offset, first_arg.begin_pos)` regardless of
      # space count.
      expect_autocorrect_parity(*klasses, "a.func   (x)\n", config)
    end

    it "removes a tab between selector and `(` on autocorrect" do
      # Tabs are also whitespace and must be eaten by the same range.
      expect_autocorrect_parity(*klasses, "a.func\t(x)\n", config)
    end
  end

  describe "nested call context" do
    it "flags a call buried inside an `if` body" do
      source = "if cond\n  a.func (x)\nend\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a call buried inside a `def` body" do
      source = "def m\n  a.func (x)\nend\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a call buried inside a block body" do
      source = "list.each do\n  a.func (x)\nend\n"
      expect_lint_parity(*klasses, source, config)
    end
  end

  describe "first-argument message" do
    it "embeds the full `(...)` source in the message, even when multiline" do
      # The MSG format is `'%<argument>s' interpreted as grouped expression.`
      # — the `(...)` source must be lifted verbatim, including any inner
      # newlines. Pinned because the wrapper reads `source[arg_start..arg_end]`
      # and a refactor of `arg_start/end` could silently slip.
      source = "a.func (\n  x\n)\n"
      expect_lint_parity(*klasses, source, config)
    end
  end

  describe "outer call's own `(`" do
    it "does NOT flag when the outer call is parenthesised even with inner spaces" do
      # `a( (b) )` — outer has its own `(` `)`, so `parenthesized?` is true
      # and the `closing_loc.is_some()` / `opening_loc.is_some()` short-circuit
      # fires. Pinned to guard the outer-paren gate against accidental
      # removal.
      expect_lint_parity(*klasses, "a( (b) )\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "a( (b) )\n", config)).to be_empty
    end
  end
end
