# frozen_string_literal: true

require "spec_helper"

# Structural guard only: rubocop-performance's three
# `autocorrect_incompatible_with` declarations (ConstantRegexp <->
# RegexpMatch, BlockGivenWithExplicitBlock -> [Lint::UnusedMethodArgument,
# Naming::BlockForwarding]) reference no class shirobai replaces, so there
# is no behavioural pair to pin — but the aligner still runs at gem load so
# a future wrapper (or core slot) cannot leave a dismissed class in any
# list.
RSpec.describe "autocorrect incompatibility alignment (performance)" do
  let(:config) { RuboCop::ConfigLoader.default_configuration }
  # `Naming/BlockForwarding` ships pending, so the per-config alignment only
  # covers it when a config enables it — as any run consulting its list
  # would have.
  let(:config_with_block_forwarding) do
    base = RuboCop::ConfigLoader.default_configuration
    hash = base.to_h
    hash["Naming/BlockForwarding"] = hash["Naming/BlockForwarding"].merge("Enabled" => true)
    RuboCop::Config.new(hash, base.loaded_path)
  end

  it "keeps rubocop-performance's prepended push working on a fresh copy per call" do
    # Non-wrapper lists are translated per config (`Inject.align_for`,
    # called from `Dispatch.bundle_token` before any correction round).
    Shirobai::Inject.align_for(config_with_block_forwarding)
    # rubocop-performance prepends a module onto
    # `Naming::BlockForwarding.singleton_class` that does
    # `super.push(Performance::BlockGivenWithExplicitBlock)`. The aligner's
    # rewritten method (BlockForwarding lists the replaced
    # Style::ArgumentsForwarding) must return a fresh copy per call — a
    # frozen array raises FrozenError here, and a shared one accumulates a
    # duplicate push per call.
    bgweb = RuboCop::Cop::Performance::BlockGivenWithExplicitBlock
    2.times do
      list = RuboCop::Cop::Naming::BlockForwarding.autocorrect_incompatible_with
      expect(list).to include(Shirobai::Cop::Style::ArgumentsForwarding)
      expect(list.count(bgweb)).to eq(1)
    end
  end

  it "leaves no replaced stock class in any enabled cop's list once the config is aligned" do
    Shirobai::Inject.align_for(config)
    registry = RuboCop::Cop::Registry.global
    replaced = Shirobai::Inject.wrapper_cops.filter_map do |cop|
      Shirobai::Inject.stock_counterpart(cop)
    end
    registry.enabled(config).each do |cop|
      stale = cop.autocorrect_incompatible_with & replaced
      expect(stale).to be_empty,
                       "#{cop.cop_name} still lists replaced stock classes: #{stale}"
    end
  end
end
