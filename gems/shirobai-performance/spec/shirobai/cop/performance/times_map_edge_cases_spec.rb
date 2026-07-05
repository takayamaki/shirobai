# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Performance/TimesMap`.
#
# The vendor spec covers the canonical `.times.map` / `.times.collect` matrix
# (literal block, block-pass, safe navigation, non-literal receiver, numbered
# and `it` parameters). Several structural quirks were uncovered by probing
# stock rubocop-performance 1.26.1 directly (.tmp/2026-07-05/probe-perf).
# Pinned here because corpus parity is disposable and a refactor could
# silently regress them:
#
# - **`$!nil?` is "has a receiver", not "not the nil literal"**: a bare
#   `times.map` is skipped, but `nil.times.map` DOES match — `nil` is a
#   present receiver node — and is flagged (and, being a `nil` literal, gets
#   no `only if` clause).
# - **`handleable_receiver?` treats `&.` / `::` as non-dots**: a non-literal
#   count is flagged only through a `.` dot. `foo&.times.map` /
#   `foo::times.map` are NOT flagged, but an int/float count is
#   (`5&.times.map`, `5::times.map`) because it satisfies the literal arm.
# - **`literal?` is a FLAT type check**: `[a].times.map` is an array literal
#   (no `only if`) even though `a` is not literal, while a PARENTHESIZED
#   receiver is a `begin` node — `(2 + 3)` and even `(1..5)` are not literals
#   and DO get the `only if` clause.
# - **Empty argument parens on `map()`** stay inside stock's replaced range
#   (`corrector.replace(map_or_collect, ...)`), so `5.times.map() { }` drops
#   the `()` on autocorrect.
# - **A `times` call that itself carries a block / block-pass** is no longer a
#   bare `(call ... :times)`, so `5.times { }.map` is not flagged.
# - **A block-pass without parens** (`5.times.map &:to_s`) is still the
#   block-pass branch and corrects to `Array.new(5, &:to_s)`.
RSpec.describe Shirobai::Cop::Performance::TimesMap do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }
  let(:klasses) { [RuboCop::Cop::Performance::TimesMap, described_class] }

  describe "the `$!nil?` receiver gate" do
    it "does not flag a receiverless `times.map`" do
      expect_lint_parity(*klasses, "times.map { |i| i }\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "times.map { |i| i }\n", config)).to be_empty
    end

    it "flags a `nil` literal receiver and corrects it (no `only if`)" do
      corrected = expect_autocorrect_parity(*klasses, "nil.times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new(nil) { |i| i }\n")
    end
  end

  describe "safe-navigation and `::` gating on `times`" do
    it "does not flag `foo&.times.map` (non-literal, `&.` is not a dot)" do
      expect_lint_parity(*klasses, "foo&.times.map { |i| i }\n", config, expect_offenses: false)
    end

    it "does not flag `foo::times.map` (non-literal, `::` is not a dot)" do
      expect_lint_parity(*klasses, "foo::times.map { |i| i }\n", config, expect_offenses: false)
    end

    it "flags `5&.times.map` (int literal satisfies the literal arm)" do
      corrected = expect_autocorrect_parity(*klasses, "5&.times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new(5) { |i| i }\n")
    end

    it "flags `5::times.map` (int literal satisfies the literal arm)" do
      corrected = expect_autocorrect_parity(*klasses, "5::times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new(5) { |i| i }\n")
    end
  end

  describe "flat `literal?` classification of the count" do
    it "adds `only if` for a parenthesized sum (a `begin` node, not a literal)" do
      stock = expect_lint_parity(*klasses, "(2 + 3).times.map { |i| i }\n", config)
      expect(stock.first[2]).to include("only if `(2 + 3)` is always 0 or more.")
    end

    it "adds `only if` for a parenthesized range (still a `begin` node)" do
      stock = expect_lint_parity(*klasses, "(1..5).times.map { |i| i }\n", config)
      expect(stock.first[2]).to include("only if `(1..5)` is always 0 or more.")
    end

    it "omits `only if` for an array literal even with a non-literal element" do
      corrected = expect_autocorrect_parity(*klasses, "[a].times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new([a]) { |i| i }\n")
      stock = lint_offenses(klasses.first, "[a].times.map { |i| i }\n", config)
      expect(stock.first[2]).not_to include("only if")
    end

    it "omits `only if` for a string literal receiver" do
      corrected = expect_autocorrect_parity(*klasses, "\"5\".times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new(\"5\") { |i| i }\n")
    end

    it "omits `only if` for a negative int literal receiver" do
      corrected = expect_autocorrect_parity(*klasses, "-5.times.map { |i| i }\n", config)
      expect(corrected).to eq("Array.new(-5) { |i| i }\n")
    end
  end

  describe "autocorrect range around `map` arguments" do
    it "drops empty argument parens `map()` inside the replaced range" do
      corrected = expect_autocorrect_parity(*klasses, "5.times.map() { |i| i }\n", config)
      expect(corrected).to eq("Array.new(5) { |i| i }\n")
    end

    it "corrects a block-pass written without parens" do
      corrected = expect_autocorrect_parity(*klasses, "5.times.map &:to_s\n", config)
      expect(corrected).to eq("Array.new(5, &:to_s)\n")
    end
  end

  describe "the `times` call shape" do
    it "does not flag when `times` carries its own block" do
      expect_lint_parity(*klasses, "5.times { }.map { |i| i }\n", config, expect_offenses: false)
    end

    it "does not flag when `times` carries a block-pass" do
      expect_lint_parity(*klasses, "5.times(&:x).map { |i| i }\n", config, expect_offenses: false)
    end

    it "does not flag when `times` takes a positional argument" do
      expect_lint_parity(*klasses, "5.times(2).map { |i| i }\n", config, expect_offenses: false)
    end
  end
end
