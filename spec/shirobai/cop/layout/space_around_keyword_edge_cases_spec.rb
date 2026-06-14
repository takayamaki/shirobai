# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: branch-hook non-firing keyword completion.
#
# `else` / `ensure` / `rescue` reach the cop through TYPED fields, so the
# generic branch hook never fires for them. shirobai completes them from their
# owning parent (BeginNode / CaseNode / CaseMatchNode), and `rescue` via
# `enter_rescue`. If any one of those completions is dropped, the corresponding
# keyword's surrounding-space offense is silently missed. The dispatch hook
# structure is exactly what a refactor is likely to disturb.
#
# Incidental before this spec (the non_ascii fixture walks `when` but never
# `else` / `ensure` / `rescue`). This pins all three at once: a
# `begin..rescue..else..ensure..end` whose `rescue`, `else` and `ensure`
# keywords each lack a trailing space. Stock and shirobai must agree on the
# offenses (one per keyword). A spaced control asserts no offense.
RSpec.describe Shirobai::Cop::Layout::SpaceAroundKeyword do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::SpaceAroundKeyword,
    Shirobai::Cop::Layout::SpaceAroundKeyword
  ]

  # `rescue` (enter_rescue), `else` and `ensure` (BeginNode parent completion)
  # each missing a trailing space: stock flags all three.
  it "flags rescue/else/ensure via owning-parent completion" do
    source = "begin\nrescue=>e\nelse(z)\nensure(q)\nend\n"
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.size).to eq(3)
    messages = stock.map { |o| o[2] }
    expect(messages).to include(
      a_string_matching(/keyword `rescue`/),
      a_string_matching(/keyword `else`/),
      a_string_matching(/keyword `ensure`/)
    )
  end

  # `else` inside a `case` reaches the cop from its CaseNode parent (not the
  # generic branch hook): a missing trailing space must still be flagged.
  it "flags a case `else` via CaseNode parent completion" do
    source = "case a\nwhen 1 then b\nelse(c)\nend\n"
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.map { |o| o[2] }).to include(a_string_matching(/keyword `else`/))
  end

  # Control: the same begin/rescue/else/ensure with proper spacing emits no
  # offense — guards against the parent completion over-firing on spaced
  # keywords.
  it "emits no offense when rescue/else/ensure are properly spaced" do
    source = "begin\nrescue => e\nelse\n  z\nensure\n  q\nend\n"
    expect_lint_parity(*klasses, source, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, source, config)).to be_empty
  end
end
