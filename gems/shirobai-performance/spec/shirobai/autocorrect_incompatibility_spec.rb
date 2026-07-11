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
  it "keeps rubocop-performance's prepended push working on a fresh copy per call" do
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

  it "leaves no dismissed stock class in any active cop's list" do
    registry = RuboCop::Cop::Registry.global
    dismissed = registry.cops.filter_map do |cop|
      next unless cop.name&.start_with?("Shirobai::")

      Shirobai::Inject.stock_counterpart(cop)
    end
    registry.cops.each do |cop|
      stale = cop.autocorrect_incompatible_with & dismissed
      expect(stale).to be_empty,
                       "#{cop.cop_name} still lists dismissed stock classes: #{stale}"
    end
  end
end
