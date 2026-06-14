# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/RequireParentheses`.
#
# The vendor spec only covers single-argument predicate sends and a handful of
# ternary cases. Real corpora hit a richer set of quirks that the vendor spec
# does not exercise — the ones below were uncovered by probing stock rubocop
# during the implementation:
#
# - Multi-arg predicates: stock checks ONLY `last_argument.operator_keyword?`,
#   so `respond_to? :foo, true && false` flags but `foo.bar? a && b, c` does NOT.
# - `(a && b)` in a ternary condition: parser-gem wraps it in a `begin` node
#   (prism `ParenthesesNode`), and `node.condition.operator_keyword?` is false,
#   so the call must NOT flag.
# - Interpolation containing an `&&`: the predicate's first/last argument is the
#   outer `dstr`, not the inner `AndNode`; no offense.
# - `def foo? a && b end`: a `def` is not a `send`; no offense (defensive — the
#   `def_node` walker must not feed `check_call`).
# - csend (`&.`) predicate: `alias on_csend on_send`, so safe-nav calls flag
#   like ordinary sends.
# - Nested context (call buried in an `if` body / `begin`...`end`): the cop
#   still visits the inner call and flags it.
#
# These cases are pinned because corpus parity is disposable (a stricter
# corpus or a refactor could silently regress them), and the vendor spec
# alone would not catch them.
RSpec.describe Shirobai::Cop::Lint::RequireParentheses do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::RequireParentheses,
    Shirobai::Cop::Lint::RequireParentheses
  ]

  describe "multi-argument predicate sends" do
    it "flags when the LAST argument is an `&&`/`||` expression" do
      expect_lint_parity(*klasses, "respond_to? :foo, true && false\n", config)
    end

    it "does NOT flag when only a non-last argument is `&&`/`||`" do
      # The first argument is `a && b` but the last is `c`; stock checks only
      # `last_argument.operator_keyword?`, so this must be zero.
      expect_lint_parity(*klasses, "foo.bar? a && b, c\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo.bar? a && b, c\n", config)).to be_empty
    end
  end

  describe "ternary edge cases" do
    it "does NOT flag when the ternary condition is parenthesized" do
      # `(a && b)` parses as `begin (a && b) end` (prism `ParenthesesNode`),
      # which is not an `AndNode`/`OrNode`, so `operator_keyword?` is false.
      expect_lint_parity(*klasses, "foo? (a && b) ? c : d\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo? (a && b) ? c : d\n", config)).to be_empty
    end

    it "does NOT flag when the ternary condition is `!and_expr`" do
      # `!(a && b)` is a unary-bang send, not an `AndNode`/`OrNode`.
      expect_lint_parity(*klasses, "foo? !(a && b) ? c : d\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo? !(a && b) ? c : d\n", config)).to be_empty
    end

    it "flags when the ternary condition is `&&`/`||`" do
      expect_lint_parity(*klasses, "foo? a && b ? c : d\n", config)
    end
  end

  describe "non-predicate / non-send shapes" do
    it "does NOT flag interpolation containing `&&`" do
      # The first argument is the outer dstr, not the inner `AndNode`.
      source = "foo? \"x \#{y && z}\"\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does NOT flag a `def` whose body looks like a flagged shape" do
      # A `def` is not a `send`; the visitor must not feed `check_call`.
      source = "def foo?(a, b)\n  a && b\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "safe navigation (csend)" do
    it "flags a csend predicate like an ordinary send" do
      expect_lint_parity(*klasses, "foo&.bar? :a && :b\n", config)
    end
  end

  describe "nested call context" do
    it "flags a predicate call inside an `if` body" do
      source = "if cond\n  foo? a && b\nend\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a predicate call inside `begin`...`end`" do
      source = "begin; foo? a && b; end\n"
      expect_lint_parity(*klasses, source, config)
    end
  end
end
