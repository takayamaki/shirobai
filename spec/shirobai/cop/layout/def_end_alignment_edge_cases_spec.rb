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
  def config_for(style)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h
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

  it "splits message range and autocorrect column on chained modifiers" do
    # `private module_function def foo`: start_of_line message names the outer
    # send (col 0), but the corrected `end` lands at the def's IMMEDIATE parent
    # send (`module_function def`, col 8). The two disagree, so stock's own
    # autocorrect does NOT converge — it parks `end` at col 8 while the message
    # keeps demanding col 0. shirobai must replicate this stuck state EXACTLY
    # (matching corrected source AND the residual offense), not "fix" it.
    src = "private module_function def foo\n  end\n"
    stock = expect_lint_parity(*klasses, src, sol_config)
    expect(stock.first[2]).to include("with `private module_function def` at 1, 0")
    corrected = expect_autocorrect_parity(*klasses, src, sol_config)
    expect(corrected).to eq("private module_function def foo\n        end\n")
    # The "corrected" source still carries the same offense in BOTH cops (stock
    # parks `end` at col 8 but still wants col 0): a stable, non-converging quirk.
    residual = expect_lint_parity(*klasses, corrected, sol_config)
    expect(residual.first[2]).to include("`end` at 2, 8")
  end

  it "splits message and autocorrect for a def argument with a receiver" do
    # `obj.foo def bar` is not a modifier (receiver present); on_def fires.
    # Message names the `def` (col 8), autocorrect aligns to the call (col 0).
    src = "obj.foo def bar\n      end\n"
    stock = expect_lint_parity(*klasses, src, sol_config)
    expect(stock.first[2]).to include("with `def` at 1, 8")
    expect(expect_autocorrect_parity(*klasses, src, sol_config))
      .to eq("obj.foo def bar\nend\n")
  end

  it "ignores endless methods (no `end` to align) under both styles" do
    src = "def foo = 42\n"
    expect_lint_parity(*klasses, src, sol_config, expect_offenses: false)
    expect_lint_parity(*klasses, src, def_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, sol_config)).to be_empty
  end
end
