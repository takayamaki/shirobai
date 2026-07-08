# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for Rails/IndexBy (Architecture B). The vendor
# spec covers the canonical each_with_object / to_h / map.to_h / Hash[] matrix,
# numbered / it params and the Ruby 2.6 gate; these pin quirks that the
# candidate prefilter + `NodeLocator` relocation could regress, all in
# differential style (same snippet through stock and shirobai):
#
# - **nested transform (`ignore_node`)**: an inner `map { }.to_h` inside an
#   outer `each_with_object` both match IndexBy, but stock reports the inner
#   offense WITHOUT autocorrecting it (`part_of_ignored_node?`). Requires the
#   outer candidate to be relocated + processed before the inner one.
# - **`map { }.to_h { |k, v| ... }`**: the outer `to_h` carries its own block
#   AND its receiver is a map block; only the map-to-h send fires (the block
#   candidate self-filters), matching stock.
# - **heredoc interior**: a candidate inside string interpolation in a heredoc
#   sits outside every ancestor's expression range, exercising NodeLocator's
#   phase-2 full-descent fallback.
# - **CRLF source**: `buffer.source != raw_source`, so the wrapper takes the
#   standalone fallback; offsets must still line up with parser-gem's index.
# - **collect alias / ::Hash / do..end block**: shape coverage through the
#   real relocation path.
RSpec.describe "Rails/IndexBy edge cases" do
  include EdgeCaseParity

  let(:config) do
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Rails/IndexBy"] = (hash["Rails/IndexBy"] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end
  let(:cops) { [RuboCop::Cop::Rails::IndexBy, Shirobai::Cop::Rails::IndexBy] }

  describe "nested transform (ignore_node cross-offense)" do
    it "reports the inner offense but does not autocorrect it" do
      src = "x.each_with_object({}) { |el, h| h[y.map { |z| [bar(z), z] }.to_h] = el }\n"
      stock = expect_lint_parity(*cops, src, config)
      # Two offenses: the outer each_with_object and the contained map.to_h.
      expect(stock.length).to eq(2)
      corrected = expect_autocorrect_parity(*cops, src, config)
      # The inner map.to_h is inside the corrected outer node, so only the
      # outer transform is rewritten (stock's ignore_node semantics).
      expect(corrected).to include("index_by")
    end
  end

  describe "`map { }.to_h { |k, v| ... }` (block + map receiver)" do
    it "fires only the map-to-h send, matching stock" do
      src = "x.map { |el| [el.to_sym, el] }.to_h { |k, v| [v, k] }\n"
      stock = expect_lint_parity(*cops, src, config)
      expect(stock.length).to eq(1)
      expect_autocorrect_parity(*cops, src, config)
    end
  end

  describe "heredoc interior (NodeLocator phase-2 fallback)" do
    it "relocates a candidate inside heredoc interpolation" do
      src = <<~'RUBY'
        puts <<~MSG
          #{x.map { |el| [el.to_sym, el] }.to_h}
        MSG
      RUBY
      expect_lint_parity(*cops, src, config)
      expect_autocorrect_parity(*cops, src, config)
    end
  end

  describe "CRLF source (standalone fallback)" do
    it "keeps offsets aligned for each_with_object" do
      expect_autocorrect_parity(
        *cops, "x.each_with_object({}) { |el, h| h[foo(el)] = el }\r\n", config
      )
    end

    it "keeps offsets aligned for map.to_h" do
      expect_autocorrect_parity(*cops, "x.map { |el| [el.to_sym, el] }.to_h\r\n", config)
    end
  end

  describe "collect alias and ::Hash" do
    it "fires for `Hash[collect { }]`" do
      expect_autocorrect_parity(*cops, "Hash[x.collect { |el| [el.to_sym, el] }]\n", config)
    end

    it "fires for `::Hash[map { }]`" do
      expect_autocorrect_parity(*cops, "::Hash[x.map { |el| [el.to_sym, el] }]\n", config)
    end
  end

  describe "safe navigation each_with_object" do
    it "fires and corrects `x&.each_with_object`" do
      expect_autocorrect_parity(
        *cops, "x&.each_with_object({}) { |el, h| h[foo(el)] = el }\n", config
      )
    end
  end

  describe "multiline do..end block" do
    it "relocates the block node through its `end`" do
      src = "x.each_with_object({}) do |el, memo|\n  memo[el.to_sym] = el\nend\n"
      expect_autocorrect_parity(*cops, src, config)
    end
  end

  describe "non-offense superset candidates" do
    it "does not fire when the value is transformed (both agree)" do
      expect_lint_parity(
        *cops, "x.each_with_object({}) { |el, h| h[el.to_sym] = foo(el) }\n", config,
        expect_offenses: false
      )
    end

    it "does not fire for `Foo::Hash[map { }]`" do
      expect_lint_parity(
        *cops, "Foo::Hash[x.map { |el| [el.to_sym, el] }]\n", config, expect_offenses: false
      )
    end
  end
end
