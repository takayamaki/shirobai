# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for Rails/IndexWith (Architecture B). Same harness
# as IndexBy; the vendor spec (wrapped in `:rails60`) covers the value-transform
# matrix, the hash-literal-value brace insertion and the numbered / it params.
# These pin the relocation + gate quirks in differential style:
#
# - **TargetRailsVersion gate**: below Rails 6.0 the cop is disabled; stock and
#   shirobai must both stay silent when the Team applies the `requires_gem`
#   gate.
# - **nested / heredoc / CRLF relocation**, as for IndexBy but on the
#   value-transform (key == element) shapes.
#
# `TargetRailsVersion` is pinned to 6.0 in the config so the Team-based
# autocorrect path sees the same gate stock does.
RSpec.describe "Rails/IndexWith edge cases" do
  include EdgeCaseParity

  def config_for(rails_version)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["AllCops"] = (hash["AllCops"] || {}).merge("TargetRailsVersion" => rails_version)
    hash["Rails/IndexWith"] = (hash["Rails/IndexWith"] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:config) { config_for(6.0) }
  let(:cops) { [RuboCop::Cop::Rails::IndexWith, Shirobai::Cop::Rails::IndexWith] }

  describe "TargetRailsVersion gate" do
    it "both stay silent below Rails 6.0" do
      # Team applies the requires_gem gate; neither cop fires on Rails 5.2.
      old = config_for(5.2)
      stock_offenses, = autocorrect_run(RuboCop::Cop::Rails::IndexWith,
                                        "x.to_h { |el| [el, foo(el)] }\n", old)
      shiro_offenses, = autocorrect_run(Shirobai::Cop::Rails::IndexWith,
                                        "x.to_h { |el| [el, foo(el)] }\n", old)
      expect(stock_offenses).to be_empty
      expect(shiro_offenses).to eq(stock_offenses)
    end

    it "both fire on Rails 6.0" do
      expect_autocorrect_parity(*cops, "x.to_h { |el| [el, foo(el)] }\n", config)
    end
  end

  describe "value-transform shapes" do
    it "each_with_object" do
      expect_autocorrect_parity(
        *cops, "x.each_with_object({}) { |el, h| h[el] = foo(el) }\n", config
      )
    end

    it "map.to_h" do
      expect_autocorrect_parity(*cops, "x.map { |el| [el, foo(el)] }.to_h\n", config)
    end

    it "Hash[map { }]" do
      expect_autocorrect_parity(*cops, "Hash[x.map { |el| [el, foo(el)] }]\n", config)
    end

    it "hash-literal value without braces gains braces" do
      corrected = expect_autocorrect_parity(*cops, "x.to_h { |el| [el, foo: el] }\n", config)
      expect(corrected).to include("index_with")
    end
  end

  describe "nested transform (ignore_node cross-offense)" do
    it "reports the inner offense but does not autocorrect it" do
      src = "x.each_with_object({}) { |el, h| h[el] = y.map { |z| [z, bar(z)] }.to_h }\n"
      stock = expect_lint_parity(*cops, src, config)
      expect(stock.length).to eq(2)
      expect_autocorrect_parity(*cops, src, config)
    end
  end

  describe "heredoc interior (NodeLocator phase-2 fallback)" do
    it "relocates a candidate inside heredoc interpolation" do
      src = <<~'RUBY'
        puts <<~MSG
          #{x.map { |el| [el, foo(el)] }.to_h}
        MSG
      RUBY
      expect_lint_parity(*cops, src, config)
    end
  end

  describe "CRLF source (standalone fallback)" do
    it "keeps offsets aligned for map.to_h" do
      expect_autocorrect_parity(*cops, "x.map { |el| [el, foo(el)] }.to_h\r\n", config)
    end
  end

  describe "safe navigation each_with_object" do
    it "fires and corrects `x&.each_with_object`" do
      expect_autocorrect_parity(
        *cops, "x&.each_with_object({}) { |el, h| h[el] = foo(el) }\n", config
      )
    end
  end

  describe "non-ASCII source offset parity" do
    # The candidate byte range is ahead of its char range behind the multibyte
    # prefix; it must round-trip through SourceOffsets before NodeLocator.
    it "matches stock offsets and autocorrect under a multibyte prefix" do
      src = "# 多バイト文字を含むコメント\nx.map { |el| [el, foo(el)] }.to_h\n"
      expect_autocorrect_parity(*cops, src, config)
    end
  end
end
