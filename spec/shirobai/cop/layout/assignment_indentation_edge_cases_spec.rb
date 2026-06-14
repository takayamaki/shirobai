# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/AssignmentIndentation`.
#
# The vendor spec exercises the basic misaligned RHS, multi-LHS, and one
# 3-line `foo =\n  bar =\n  baz` chain, but it does NOT pin the following
# stock quirks that the probe surfaced. Each is a differential against the
# 1.87-pinned stock; a refactor of either the AST callback set, the
# `leftmost_multiple_assignment` walk, or the `AlignmentCorrector` hand-off
# could silently regress them.
#
#   1. The `CheckAssignment` mixin fires `on_send` too: an attribute setter
#      (`x.foo = ...`) and an index setter (`x[0] = ...`) both have
#      `node.loc.operator` set (`=`) and must be checked. Prism exposes both
#      as `CallNode#equal_loc` (not `operator_loc`, which is for chained call
#      operators), so a wrapper using the wrong field would skip them.
#   2. Compound-assignment operators (`+=` / `||=` / `&&=`) likewise carry
#      `node.loc.operator` and must be checked. Each prism variant routes
#      through a different `*_operator_write_node` / `*_or_write_node` /
#      `*_and_write_node` accessor pair (`binary_operator_loc` vs
#      `operator_loc`); missing any one would silently skip that family.
#   3. Comparison `===` is a `send` whose `loc.operator` is nil (no
#      `equal_loc` on the prism `CallNode`), so stock returns early; shirobai
#      must match by skipping non-setter calls.
#   4. The autocorrect column delta is signed: `a =\n            if b ; end`
#      (RHS over-indented to col 12) must shrink to col 2 with `column_delta
#      = -10`. Stock's `AlignmentCorrector#correct` handles the negative
#      branch (remove `column_delta.abs` of leading whitespace); the wrapper
#      must hand the SAME signed delta plus the located `Parser::AST::Node`
#      so the negative branch fires byte-for-byte.
#   5. A `Parser::AST::Node` is needed (not just a range) because
#      `AlignmentCorrector#inside_string_ranges` and `block_comment_within?`
#      both descend the node. Handing the bare `Parser::Source::Range` would
#      silently disable those taboos.
#   6. An assignment that is the LAST ARGUMENT of a call (`f a =\n  b`) must
#      still be checked through its own `on_lvasgn` callback (the outer call
#      `f` is a regular send with no `equal_loc` and returns early). The
#      inner `a = b` then computes `leftmost = a` (its parent `f` is NOT
#      `assignment?` so the climb stops), `base = column(a)`, `expected =
#      base + 2`, and the offense aligns `b` to col 4.
RSpec.describe Shirobai::Cop::Layout::AssignmentIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::AssignmentIndentation,
    Shirobai::Cop::Layout::AssignmentIndentation
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "checks an attribute setter (`x.foo = ...`) via on_send equal_loc" do
    src = "x.foo =\nif b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("x.foo =\n  if b ; end\n")
  end

  it "checks an index setter (`x[0] = ...`) via on_send equal_loc" do
    src = "x[0] =\nif b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("x[0] =\n  if b ; end\n")
  end

  it "checks `+=` (operator_write — binary_operator_loc)" do
    src = "a +=\nif b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("a +=\n  if b ; end\n")
  end

  it "checks `||=` (or_write — operator_loc)" do
    src = "a ||=\nif b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("a ||=\n  if b ; end\n")
  end

  it "checks `&&=` (and_write — operator_loc)" do
    src = "a &&=\nif b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("a &&=\n  if b ; end\n")
  end

  it "ignores `===` (a `send` with no equal_loc)" do
    src = "a ===\nif b ; end\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, config)).to be_empty
  end

  it "applies a negative column delta when the RHS is over-indented" do
    src = "a =\n            if b ; end\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("a =\n  if b ; end\n")
  end

  it "checks an inner asgn when it is the last argument of a call" do
    # The outer `f` send has no equal_loc (returns early). The inner `a = b`
    # lvasgn has operator at col 4, rhs `b` at line 2 col 2; leftmost is
    # `a = b` itself (its parent send `f` is not `assignment?`), so base=2
    # and expected col 4. delta=2 shifts `b` right two columns.
    src = "f a =\n  b\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("f a =\n    b\n")
  end

  it "shifts every line of a multi-line RHS together" do
    # The RHS spans three lines; AlignmentCorrector#correct walks every line
    # of the RHS range and shifts each by `column_delta`. shirobai relocates
    # the parser-gem RHS node (begin = if-keyword col 0 line 2) so the
    # corrector walks the same three lines.
    src = "a =\n(if b\n end)\n"
    expect_lint_parity(*klasses, src, config)
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("a =\n  (if b\n   end)\n")
  end

  it "preserves the comparison-operator skip even for chained `===` assignment" do
    # `a === b === c` is a chain of `===` sends, none with equal_loc. No
    # offense regardless of line layout.
    src = "a ===\nb ===\nc\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end
end
