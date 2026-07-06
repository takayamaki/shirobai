# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for the Architecture-B
# `DeprecatedActiveModelErrorsMethods` wrapper. The vendor spec covers the
# canonical matrix (both file kinds, both Rails gates); these pin quirks the
# differential harness must keep byte-identical:
#
# - **`.keys` node is the offense, not the enclosing `.include?`**: the
#   candidate range must relocate to the `.keys` send.
# - **version gate**: `keys` / `values` / `to_h` / `to_xml` fire only on
#   Rails >= 6.1 (`INCOMPATIBLE_METHODS`); `<<` / `clear` / `[]=` fire always.
# - **uncorrectable receivers**: `errors.details[...] << ...` is an offense
#   with NO correction (`skip_autocorrect?`), and `[]=` never corrects.
# - **non-ASCII / CRLF**: byte-vs-char offsets and the standalone fallback.
RSpec.describe "Rails/DeprecatedActiveModelErrorsMethods edge cases" do
  include EdgeCaseParity

  COP_NAME = "Rails/DeprecatedActiveModelErrorsMethods"

  def rails_config(version)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["AllCops"] = (hash["AllCops"] || {}).merge("TargetRailsVersion" => version)
    # Force-enable: the cop ships `Enabled: pending`, which the Team treats as
    # disabled unless pending cops are turned on.
    hash[COP_NAME] = (hash[COP_NAME] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:config) { rails_config(6.1) }
  let(:pair) do
    [
      RuboCop::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      Shirobai::Cop::Rails::DeprecatedActiveModelErrorsMethods
    ]
  end

  it "corrects `errors.keys` to `.attribute_names`, offense on the keys node" do
    corrected = expect_autocorrect_parity(
      *pair, "user.errors.keys.include?(:name)\n", config
    )
    expect(corrected).to eq("user.errors.attribute_names.include?(:name)\n")
  end

  it "corrects a << on errors[...] to .add(key, value)" do
    corrected = expect_autocorrect_parity(*pair, "user.errors[:name] << 'msg'\n", config)
    expect(corrected).to eq("user.errors.add(:name, 'msg')\n")
  end

  it "flags errors.details[...] << but does not correct it" do
    stock = expect_lint_parity(*pair, "user.errors.details[:name] << {}\n", config)
    expect(stock.first[4]).to be(false) # correctable? == false
  end

  it "flags errors[...] = but does not correct it" do
    stock = expect_lint_parity(*pair, "user.errors[:name] = []\n", config)
    expect(stock.first[4]).to be(false)
  end

  it "does not flag errors.messages[...].keys" do
    expect_lint_parity(*pair, "user.errors.messages[:name].keys\n", config, expect_offenses: false)
  end

  it "flags an errors call nested in heredoc interpolation" do
    # The send lives in a heredoc BODY, outside the expression range of every
    # ancestor up to the root; `NodeLocator` must still relocate it (regression
    # from a real discourse divergence).
    source = "log(<<~TEXT)\n  errors: \#{e.record.errors.to_h}\nTEXT\n"
    expect_lint_parity(*pair, source, config)
  end

  context "on Rails <= 6.0" do
    let(:config) { rails_config(6.0) }

    it "does not flag the incompatible `keys` method" do
      expect_lint_parity(*pair, "user.errors.keys\n", config, expect_offenses: false)
    end

    it "still flags `<<` (correctable regardless of version)" do
      expect_lint_parity(*pair, "user.errors[:name] << 'msg'\n", config)
    end
  end

  it "matches stock through a non-ASCII prefix" do
    corrected = expect_autocorrect_parity(
      *pair, "# 多バイトのコメント\nuser.errors[:name].clear\n", config
    )
    expect(corrected).to eq("# 多バイトのコメント\nuser.errors.delete(:name)\n")
  end

  it "matches stock on CRLF source (standalone fallback path)" do
    # `buffer.source != raw_source` -> the wrapper takes its standalone entry.
    # Whatever stock does with the CRLF, shirobai must match byte for byte
    # (the differential is the assertion).
    corrected = expect_autocorrect_parity(*pair, "user.errors[:name] << 'msg'\r\n", config)
    expect(corrected).to include("user.errors.add(:name, 'msg')")
  end
end
