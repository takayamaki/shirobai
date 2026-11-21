# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/DefEndAlignment`.
#
# The vendor spec exercises `private def` / `foo def` and the two
# `EnforcedStyleAlignWith` styles, but it does NOT pin several quirks the stock
# real-machine probe surfaced, all of which a refactor could silently regress:
#
#   1. `def self.foo` (`on_defs`): the message names the `def` KEYWORD (never
#      `self.foo`), and both styles align to the `def` keyword column.
#   2. The `start_of_line` vs `def` style divergence on a `private def`: the
#      former aligns to the `private` line start (col 0), the latter to the
#      inner `def` keyword (col 8) — same source, opposite corrected output.
#   3. CHAINED modifiers (`private module_function def foo`) under
#      `start_of_line`: the offense MESSAGE names the OUTERMOST send's line
#      range (`private module_function def`, col 0) while the autocorrect COLUMN
#      comes from the def's IMMEDIATE parent send (`module_function def`, col 8).
#      The message and the corrected indentation deliberately disagree — a stock
#      quirk (`add_offense_for_misalignment` uses the firing callback's range,
#      `autocorrect` uses `node.parent`). The autocorrect must still converge.
#   4. A def that is a call argument WITH a receiver (`obj.foo def bar`) is NOT a
#      `def_modifier?` (receiver present), so `on_def` fires; under
#      `start_of_line` the message names the `def` (col 8) but the autocorrect
#      aligns to the enclosing call (`node.parent`, col 0).
#
# Corpus-only / probe-only before this spec; pinned here as differential
# regressions against the 1.87-pinned stock.
RSpec.describe Shirobai::Cop::Layout::DefEndAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::DefEndAlignment,
    Shirobai::Cop::Layout::DefEndAlignment
  ]

  # A config with `EnforcedStyleAlignWith` set to `style` on top of defaults.
  # `Config#to_h` returns the default configuration's INTERNAL hash, so it must
  # be duped before the key is reassigned — mutating it in place leaks the style
  # into every later spec that reads the (identity-memoized) default.
  def config_for(style)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Layout/DefEndAlignment"] =
      hash["Layout/DefEndAlignment"].merge("EnforcedStyleAlignWith" => style)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:sol_config) { config_for("start_of_line") }
  let(:def_config) { config_for("def") }

  it "names the `def` keyword (not `self.foo`) for an `on_defs` misalignment" do
    src = "def self.foo\n    end\n"
    stock = expect_lint_parity(*klasses, src, sol_config)
    expect(stock.first[2]).to include("with `def` at 1, 0")
    expect_autocorrect_parity(*klasses, src, sol_config)
  end

  it "diverges between `start_of_line` and `def` styles on `private def`" do
    src = "private def foo\n            end\n"

    sol = expect_lint_parity(*klasses, src, sol_config)
    expect(sol.first[2]).to include("with `private def` at 1, 0")
    expect(expect_autocorrect_parity(*klasses, src, sol_config))
      .to eq("private def foo\nend\n")

    dfn = expect_lint_parity(*klasses, src, def_config)
    expect(dfn.first[2]).to include("with `def` at 1, 8")
    expect(expect_autocorrect_parity(*klasses, src, def_config))
      .to eq("private def foo\n        end\n")
  end

  # Up to 1.92.2 `start_of_line` aligned the corrected `end` with the def node
  # parent send, while the message names the outermost modifier. That split
  # the two for chained modifiers (stock parked `end` at col 8 and never
  # converged) and for a def argument of a call with a receiver. 1.92.3
  # (rubocop#16251) aligns with `outermost_def_modifier`, so both now agree.
  it "aligns `end` with the outermost send on chained modifiers" do
    src = "private module_function def foo\n  end\n"
    stock = expect_lint_parity(*klasses, src, sol_config)
    expect(stock.first[2]).to include("with `private module_function def` at 1, 0")
    expect(expect_autocorrect_parity(*klasses, src, sol_config))
      .to eq("private module_function def foo\nend\n")
  end

  it "aligns `end` with the `def` for a def argument with a receiver" do
    # `obj.foo def bar` is not a modifier (receiver present); on_def fires.
    src = "obj.foo def bar\n      end\n"
    stock = expect_lint_parity(*klasses, src, sol_config)
    expect(stock.first[2]).to include("with `def` at 1, 8")
    expect(expect_autocorrect_parity(*klasses, src, sol_config))
      .to eq("obj.foo def bar\n        end\n")
  end

  # rubocop-ast `def_modifier` looks at the first argument only: more
  # arguments, a block argument or a block keep the send a modifier.
  it "treats a send whose first argument is the def as a modifier" do
    src = <<~RUBY
      foo def bar
        end, 1
      foo(def baz
        end) { }
      foo(def qux
        end, &b)
      foo(x, bar(def quux
        end))
      foo(bar(
        def corge
          end), 1)
    RUBY
    [sol_config, def_config].each do |cfg|
      expect_lint_parity(*klasses, src, cfg)
      expect_autocorrect_parity(*klasses, src, cfg)
    end
  end

  it "ignores endless methods (no `end` to align) under both styles" do
    src = "def foo = 42\n"
    expect_lint_parity(*klasses, src, sol_config, expect_offenses: false)
    expect_lint_parity(*klasses, src, def_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, sol_config)).to be_empty
  end
end

# 1.92: a single-line `def` under a modifier is skipped before `ignore_node`,
# so its own `on_def` runs (and accepts it). Chained modifiers and a line
# continuation are not in the vendor spec.
RSpec.describe Shirobai::Cop::Layout::DefEndAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::DefEndAlignment,
    Shirobai::Cop::Layout::DefEndAlignment
  ]

  it "matches stock for single-line defs under modifiers" do
    cfg = RuboCop::ConfigLoader.default_configuration
    source = "foo \\\n  def test; end\n" \
             "private module_function \\\n  def m; end\n" \
             "private def n\n  1\n    end\n"
    expect_lint_parity(*klasses, source, cfg)
    expect_autocorrect_parity(*klasses, source, cfg)
  end
end
