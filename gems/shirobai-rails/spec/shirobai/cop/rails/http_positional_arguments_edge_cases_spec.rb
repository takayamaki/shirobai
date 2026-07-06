# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for the Architecture-B `HttpPositionalArguments`
# wrapper. The vendor spec covers the canonical offense/correction matrix;
# these pin quirks the differential harness (stock vs shirobai, fresh cop per
# file) must keep byte-identical and that a refactor could silently regress:
#
# - **candidate relocation**: the wrapper reaches stock's `on_send` only if the
#   Rust candidate range relocates to the exact parser send node (session
#   arg, parentheses, multiline).
# - **stock guards run on the parser AST**: routing block, `Rack::Test::Methods`,
#   kwsplat / forwarded-arg all self-filter in the wrapper.
# - **non-ASCII**: byte offsets ahead of char offsets must go through
#   `SourceOffsets` before hitting `Parser::Source::Range`.
# - **CRLF**: `buffer.source != raw_source`, so the wrapper falls back to its
#   standalone entry scanning `buffer.source`; offsets must still line up.
RSpec.describe "Rails/HttpPositionalArguments edge cases" do
  include EdgeCaseParity

  # Rails >= 5 gate: stub railties in the target so the `requires_gem`
  # (`TargetRailsVersion`) gate the Team applies is satisfied on both sides.
  let(:config) do
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["AllCops"] = (hash["AllCops"] || {}).merge("TargetRailsVersion" => 7.0)
    hash["Rails/HttpPositionalArguments"] =
      (hash["Rails/HttpPositionalArguments"] || {}).merge("Enabled" => true)
    cfg = RuboCop::Config.new(hash, default.loaded_path)
    cfg.define_singleton_method(:gem_versions_in_target) { { "railties" => Gem::Version.new("7.0.0") } }
    cfg
  end

  let(:pair) do
    [RuboCop::Cop::Rails::HttpPositionalArguments, Shirobai::Cop::Rails::HttpPositionalArguments]
  end

  it "adds a session keyword when a third arg is present" do
    corrected = expect_autocorrect_parity(
      *pair,
      "get some_path(profile.id), {}, 'HTTP_REFERER' => p_url(p.id).to_s\n",
      config
    )
    expect(corrected).to eq("get some_path(profile.id), session: { 'HTTP_REFERER' => p_url(p.id).to_s }\n")
  end

  it "maintains parentheses when autocorrecting" do
    corrected = expect_autocorrect_parity(*pair, "post(:user_attrs, id: 1)\n", config)
    expect(corrected).to eq("post(:user_attrs, params: { id: 1 })\n")
  end

  it "converts a multiline positional hash joining pairs with a comma" do
    source = "patch :update,\n      id: @user.id,\n      ac: { a: 1 }\n"
    expect_autocorrect_parity(*pair, source, config)
  end

  it "does not fire inside a routing block" do
    expect_lint_parity(
      *pair,
      "routes do\n  get :list, on: :collection\nend\n",
      config,
      expect_offenses: false
    )
  end

  it "does not fire when Rack::Test::Methods is included" do
    expect_lint_parity(
      *pair,
      "include Rack::Test::Methods\n\nget :create, user_id: @user.id\n",
      config,
      expect_offenses: false
    )
  end

  it "does not fire for a kwsplat hash" do
    expect_lint_parity(*pair, "get :nothing, **args\n", config, expect_offenses: false)
  end

  it "matches stock through a non-ASCII prefix" do
    corrected = expect_autocorrect_parity(
      *pair,
      "# 多バイトのコメント\nget :new, user_id: @user.id\n",
      config
    )
    expect(corrected).to eq("# 多バイトのコメント\nget :new, params: { user_id: @user.id }\n")
  end

  it "matches stock on CRLF source (standalone fallback path)" do
    # `buffer.source != raw_source` -> the wrapper takes its standalone entry;
    # the differential is the assertion.
    corrected = expect_autocorrect_parity(*pair, "get :new, id: 1\r\n", config)
    expect(corrected).to include("get :new, params: { id: 1 }")
  end
end
