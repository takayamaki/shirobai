# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/UnreachableCode`.
#
# The vendor spec (266 examples) is thorough but every fixture is wrapped in
# `def something; array.each do |item| ... end; end` to give the multi-
# statement body a parent. Several structural quirks of the shirobai
# implementation are not exercised by that wrapping and the corpus parity is
# close to a no-op (Mastodon / Redmine produce zero offenses for this cop,
# Rubocop self has 2), so a refactor of the visit machinery would not surface
# any regression via the corpus path. The cases below pin behaviours the
# vendor spec does not reach.
#
# - **Top-level `ProgramNode` statements**: the generated `visit_program_node`
#   default jumps straight to `visit_statements_node` without going through
#   `Visit::visit`, so `visit_branch_node_enter` (which `dispatch::Rule::enter`
#   piggy-backs on) is NEVER called for the top-level statements list. The
#   bundle path must explicitly handle the `ProgramNode` hook; otherwise a
#   bare `return` / `raise` at the file top wraps NOTHING and the cop silently
#   drops the offense. Cf. `crates/shirobai-core/src/rules/void.rs` which has
#   the same `as_program_node()` hook in `Rule::enter`.
# - **Explicit `begin..end` (kwbegin) vs implicit multi-statement body
#   (parser `:begin`)**: in prism these are the `BeginNode` (whose
#   `statements()` field is the "top section") and the bare `StatementsNode`
#   respectively. We must process both AND avoid double-firing on the inner
#   `StatementsNode` of a `BeginNode` (the shared walk visits it again right
#   after the BeginNode enter). The vendor spec covers begin..end with raise
#   but only inside a wrapped `each` block.
# - **Brace-style block (`d.instance_eval { raise; bar }`)**: prism uses a
#   single `BlockNode` for both `do..end` and `{ ... }` forms, distinguished
#   only by the opening/closing tokens. The vendor spec covers `do..end`
#   only.
# - **Custom `instance_eval` redefinition inside a `def`**: stock's
#   `redefinable_flow_method?` list does NOT include `instance_eval`, so a
#   `def raise` in a sibling position registers but `def instance_eval` does
#   not. Pinned to guard against a future refactor that conflates them.
RSpec.describe Shirobai::Cop::Lint::UnreachableCode do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::UnreachableCode,
    Shirobai::Cop::Lint::UnreachableCode
  ]

  describe "top-level ProgramNode statements" do
    it "flags a `bar` after a top-level `return`" do
      # The bare top-level expressions `return\nbar` form a parser `:begin`
      # node at file scope; the prism analogue is `ProgramNode.statements()`,
      # which the shared-walk dispatch visits via a path that bypasses
      # `visit_branch_node_enter` (see `visit_program_node` in the generated
      # bindings). The Rule must handle the `ProgramNode` hook explicitly or
      # the top-level offense is silently dropped.
      source = "return\nbar\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a `bar` after a top-level `raise`" do
      source = "raise\nbar\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "flags a `bar` after a top-level `Kernel.exit`" do
      # Explicit Kernel receiver is always a flow command, even at top level.
      source = "Kernel.exit\nbar\n"
      expect_lint_parity(*klasses, source, config)
    end
  end

  describe "explicit `begin..end` vs implicit multi-statement body" do
    it "flags `bar` inside a top-level `begin...end` after `raise`" do
      # The `begin..end` is a parser `:kwbegin` -> prism `BeginNode`; its
      # `statements()` is the section before any rescue/else/ensure. The
      # vendor spec wraps these in a method body; pin a bare top-level
      # variant so the BeginNode-vs-StatementsNode distinction is exercised
      # directly.
      source = "begin\n  raise\n  bar\nend\n"
      expect_lint_parity(*klasses, source, config)
    end

    it "does NOT double-fire on the inner StatementsNode of a `begin..end`" do
      # If we process BOTH the BeginNode AND its child StatementsNode the
      # offense would be reported twice (same fixture as above). The
      # `processed_statements` set must dedupe.
      source = "begin\n  raise\n  bar\nend\n"
      offenses = lint_offenses(klasses.last, source, config)
      expect(offenses.size).to eq(1)
    end
  end

  describe "block forms for `instance_eval`" do
    it "silences a bare `raise` inside a brace-style instance_eval block" do
      # Stock's `instance_eval_block?` is `any_block_type? &&
      # method?(:instance_eval)`. In prism `{ ... }` and `do..end` are both
      # `BlockNode`; the parent CallNode's name decides whether we're in
      # `instance_eval`. The vendor spec exercises `do..end` only.
      source = "d.instance_eval { raise; bar }\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "still flags an outer raise after a brace-style instance_eval" do
      source = "d.instance_eval { raise; bar }\nraise\nfoo\n"
      expect_lint_parity(*klasses, source, config)
    end
  end

  describe "redefinable-method scope" do
    it "does NOT treat `def instance_eval` as a redefinable flow method" do
      # `instance_eval` is not in the redefinable list (only raise/fail/throw/
      # exit/exit!/abort are). A sibling `def instance_eval` must not register
      # — and bare `instance_eval` calls are not flow commands either way, so
      # the case is degenerate, but pin the scope so a future refactor doesn't
      # accidentally widen the matcher.
      source = "def instance_eval; end\nraise\nbar\n"
      # `raise` is still a flow command because `instance_eval` is not
      # registered as a redefinition of `raise`.
      expect_lint_parity(*klasses, source, config)
    end

    it "registers only when the def name is a redefinable flow method" do
      # `def something_else` registers nothing -> a subsequent bare `raise`
      # is still flow -> `bar` is unreachable.
      source = "def something_else; end\nraise\nbar\n"
      expect_lint_parity(*klasses, source, config)
    end
  end
end
