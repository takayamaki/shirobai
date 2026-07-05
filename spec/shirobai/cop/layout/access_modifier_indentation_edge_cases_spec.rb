# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/AccessModifierIndentation`.
#
# The vendor spec covers misaligned `private` / `protected` / `module_function`
# under both styles, but it does NOT pin several quirks the stock real-machine
# probe surfaced — all corner cases where the prism-AST differs from the
# parser-AST stock walks, and where a refactor could silently flip behaviour:
#
#   1. `class A; private; end` (single-statement body): parser body is the
#      bare `(send nil? :private)` (no `(begin ...)` wrap), so `body.begin_type?`
#      is FALSE and stock emits nothing. Prism, however, wraps every body in
#      `StatementsNode` even for a single statement — the wrapper must
#      length-gate to stay no-offense here.
#   2. `class A; begin; private; def x; end; end; end` (explicit
#      `begin..end`): the parser body is `(kwbegin (begin ...))` and stock's
#      `body.begin_type?` is FALSE on the kwbegin, so no offense. Prism gives
#      `BeginNode` (not `StatementsNode`) so the body-type guard skips it
#      identically. Pinned to lock the rationale.
#   3. `class A; if cond; private; ...; end; end`: the body is an `(if ...)`
#      node, not a `(begin ...)`; `each_child_node(:send)` never reaches the
#      modifier and stock emits nothing. The prism shape is `IfNode` — same
#      result, pinned here.
#   4. `class A; private; def x; end; end` (one-line; `private` shares the
#      class header line): the parser body IS a `(begin ...)` BUT
#      `same_line?(node, modifier)` skips on-header modifiers, so stock emits
#      nothing. shirobai mirrors that via `header_line` comparison.
#   5. `func do; X = 1; private; ...; end` (arbitrary block; aliased
#      `on_block` in stock): both parser and prism reach the block body, but
#      prism BlockNode also covers `LambdaNode` (`-> { ... }`) — the wrapper
#      has to handle the lambda form too, since stock's `on_block` catches a
#      lambda literal (it's a parser-`block` there). Pinned for both.
#   6. `class A; private :foo; def foo; end; end`: `private :foo` has an
#      argument so `bare_access_modifier?` returns FALSE; stock emits nothing.
#      shirobai's `arguments.is_some()` filter mirrors that.
#
# Corpus-only / probe-only before this spec; pinned here as differential
# regressions against the 1.87-pinned stock.
RSpec.describe Shirobai::Cop::Layout::AccessModifierIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::AccessModifierIndentation,
    Shirobai::Cop::Layout::AccessModifierIndentation
  ]

  # `Config#to_h` returns the default configuration's INTERNAL hash, so it must
  # be duped before the key is reassigned — mutating it in place leaks the style
  # into every later spec that reads the (identity-memoized) default.
  let(:config) do
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Layout/AccessModifierIndentation"] =
      hash["Layout/AccessModifierIndentation"].merge("EnforcedStyle" => "indent")
    RuboCop::Config.new(hash, default.loaded_path)
  end

  it "ignores a single-statement class body (no `begin` wrap in parser)" do
    src = "class A\nprivate\nend\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "ignores an explicit `begin..end` class body (kwbegin, not begin)" do
    src = "class A\n  begin\n  private\n    def x; end\n  end\nend\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "ignores a modifier sitting inside an `if` branch (not direct body child)" do
    src = "class A\n  if true\n    private\n  end\nend\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "skips a modifier on the same line as the class header" do
    src = "class A; private; def foo; end; end\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "inspects modifiers inside `do..end` and `{...}` blocks (parser `block` node)" do
    do_src = "func do\n  X = 1\nprivate\n  def x; end\nend\n"
    expect(expect_autocorrect_parity(*klasses, do_src, config))
      .to eq("func do\n  X = 1\n  private\n  def x; end\nend\n")

    brace_src = "Test = Class.new {\n  X = 1\nprivate\n  def x; end\n}\n"
    expect(expect_autocorrect_parity(*klasses, brace_src, config))
      .to eq("Test = Class.new {\n  X = 1\n  private\n  def x; end\n}\n")
  end

  it "inspects modifiers inside a lambda block (`-> { ... }`)" do
    # parser surfaces `-> { ... }` as a `block` node (and `on_block` catches
    # it); prism splits it into `LambdaNode`, so the wrapper must hook both.
    src = "-> {\n  X = 1\nprivate\n  def x; end\n}\n"
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("-> {\n  X = 1\n  private\n  def x; end\n}\n")
  end

  it "ignores a modifier with arguments (`private :foo` is not bare)" do
    src = "class A\n  X = 1\n  private :foo\n  def foo; end\nend\n"
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end

  it "inspects each nested class independently" do
    src = "class Outer\n\n  class Inner\n\n  private\n\n    def a; end\n  end\n\n  protected\n\n  def test; end\nend\n"
    stock = expect_lint_parity(*klasses, src, config)
    expect(stock.first[2]).to include("Indent access modifiers like `private`")
    expect(expect_autocorrect_parity(*klasses, src, config))
      .to eq("class Outer\n\n  class Inner\n\n    private\n\n    def a; end\n  end\n\n  protected\n\n  def test; end\nend\n")
  end
end
