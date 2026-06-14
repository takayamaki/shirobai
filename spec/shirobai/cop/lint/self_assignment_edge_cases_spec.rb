# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/SelfAssignment`.
#
# The vendor spec covers the common shapes well, but a few quirks emerged
# from probing stock during the implementation that no vendor case pins
# explicitly. These specs lock them in as differential against stock — corpus
# parity is disposable, and a refactor could silently regress them.
#
# - `Foo, Bar = Foo, Bar` (masgn over constants): stock's
#   `ASSIGNMENT_TYPE_TO_RHS_TYPE` has no `casgn` entry, so the pair never
#   matches. NO offense.
# - `A::B = A::B` (constant path lhs and rhs): prism splits this into a
#   `ConstantPathWriteNode`, not `ConstantWriteNode`. Visitor must cover
#   both; offense.
# - `foo&.bar = foo.bar` (csend-vs-send setter mismatch): stock's `==` is
#   structural, so `lvar foo` (receiver of both) compares equal regardless
#   of the operator on the outer setter. Offense.
# - `foo.[]= ` with NO arguments: stock's `handle_key_assignment` bails on
#   missing `value_node`. NO offense.
# - `foo[bar] = foo[bar]` where `bar` is a method call: stock excludes any
#   key argument that is `call_type?`, so this key is excluded. NO offense.
# - `singleton[] = singleton[]` (zero-arg `[]=`): the two args lists are
#   both empty, `==` is true; offense.
# - `Foo = bar` (rhs not const): `on_casgn` bails on
#   `rhs&.const_type?`. NO offense.
# - `AllowRBSInlineAnnotation: true` semantics: the same-line `#: ...` form
#   excludes the offense for single-line lvasgn, but masgn's `first_lhs`
#   does not associate with a comment on the wrap-end line of a multi-line
#   masgn (stock's `associate_by_identity` only ties decorating comments to
#   the node's last line). NO RBS exemption for multi-line masgn.
RSpec.describe Shirobai::Cop::Lint::SelfAssignment do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::SelfAssignment,
    Shirobai::Cop::Lint::SelfAssignment
  ]

  describe "constant assignment quirks" do
    it "does NOT flag a masgn over constants" do
      # `Foo, Bar = Foo, Bar` -> stock's ASSIGNMENT_TYPE_TO_RHS_TYPE has no
      # casgn entry, so multi_pair_matches is false for every pair.
      source = "Foo, Bar = Foo, Bar\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "flags a constant path self-assignment (`A::B = A::B`)" do
      # prism splits this into ConstantPathWriteNode (not ConstantWriteNode);
      # the visitor must cover both shapes.
      expect_lint_parity(*klasses, "A::B = A::B\n", config)
    end

    it "does NOT flag a constant path with different namespace (`Foo::Bar = ::Foo::Bar`)" do
      source = "Foo::Bar = ::Foo::Bar\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does NOT flag `Foo = bar` (rhs is not a constant)" do
      source = "Foo = bar\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "attribute and `[]=` quirks" do
    it "flags `foo&.bar = foo.bar` (mismatched send/csend setter still matches receivers)" do
      # stock compares receivers by AST `==`; `lvar foo` ≡ `lvar foo` regardless
      # of whether the outer setter is `send` or `csend`. Offense.
      expect_lint_parity(*klasses, "foo&.bar = foo.bar\n", config)
    end

    it "does NOT flag `foo.[]= ` with no arguments" do
      # stock's `handle_key_assignment` reads `value_node = node.last_argument`;
      # with zero args there is no value_node and the method call below
      # (`value_node.method?(:[])`) would raise — stock guards by
      # `value_node.respond_to?(:method?)`. Result: no offense.
      source = "foo.[]=\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does NOT flag `foo[bar] = foo[bar]` when the key is a method call" do
      # `bar` (no receiver, no `=`) is a `send` node, so it's `call_type?`,
      # and stock's `node_arguments.none?(&:call_type?)` rejects the case.
      source = "foo[bar] = foo[bar]\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "flags zero-arg `singleton[] = singleton[]`" do
      # The key portion is empty; stock takes `node_arguments = []` and
      # `value_node.arguments = []`, which are equal — offense.
      expect_lint_parity(*klasses, "singleton[] = singleton[]\n", config)
    end

    it "flags `[]=` self-assignment with multi-key args" do
      expect_lint_parity(*klasses, "matrix[1, 2] = matrix[1, 2]\n", config)
    end

    it "does NOT flag `matrix[1, foo] = matrix[1, foo]` when one key is a method call" do
      source = "matrix[1, foo] = matrix[1, foo]\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "AllowRBSInlineAnnotation semantics" do
    let(:allow_config) do
      RuboCop::Config.new(
        "Lint/SelfAssignment" => { "AllowRBSInlineAnnotation" => true }
      )
    end

    it "excludes single-line lvasgn with `#: type` annotation" do
      source = "foo = foo #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, allow_config)).to be_empty
    end

    it "still flags single-line lvasgn with a plain `#` comment" do
      expect_lint_parity(*klasses, "foo = foo # plain comment\n", allow_config)
    end

    it "STILL flags multi-line masgn where `#: type` decorates the wrap-end line" do
      # `associate_by_identity` only ties decorating comments to a node's
      # last_line. The masgn ENDS on line 2 (with `foo, bar`), but stock
      # checks the FIRST LHS (`foo` lvasgn on line 1) for the annotation;
      # the comment binds to the line-2 lvar, not the line-1 lvasgn —
      # so RBS exemption does NOT apply. Offense.
      source = "foo, bar =\n  foo, bar #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config)
    end

    it "EXCLUDES multi-line lvasgn where the rhs (the decorator target) is on the comment line" do
      # The rhs (`lvar foo`) ends on line 2, where `#: Integer` decorates
      # the lvar node — `awc[rhs]` hits, and the offense is dropped.
      source = "foo =\n  foo #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, allow_config)).to be_empty
    end

    it "STILL flags when a leading statement on the same line ends before the comment" do
      # `baz = 1; foo = foo #: Integer` — `associate_by_identity` ties the
      # comment to `int 1` (the first node whose last_line == the comment's
      # line), NOT to the rhs `lvar foo`. The second statement is still
      # flagged.
      source = "baz = 1; foo = foo #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config)
    end

    it "EXCLUDES when self-asgn is FIRST on a line whose later statement ends with `#:`" do
      # `foo = foo; baz = 1 #: Integer` — the comment binds to the rhs of
      # `foo = foo` (the first lvar that ends on line 1), so the cop
      # excludes that offense.
      source = "foo = foo; baz = 1 #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, allow_config)).to be_empty
    end

    it "excludes attribute setter with `#: type`" do
      source = "foo.bar = foo.bar #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, allow_config)).to be_empty
    end

    it "excludes `[]=` self-assignment with `#: type`" do
      source = "foo[\"bar\"] = foo[\"bar\"] #: Integer\n"
      expect_lint_parity(*klasses, source, allow_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, allow_config)).to be_empty
    end
  end

  describe "nested call context" do
    it "flags a self-assignment inside a method body" do
      source = "def m\n  foo = foo\nend\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a self-assignment inside a block body" do
      source = "items.each do |x|\n  foo = foo\nend\n"
      expect_lint_parity(*klasses, source, config)
    end
  end
end
