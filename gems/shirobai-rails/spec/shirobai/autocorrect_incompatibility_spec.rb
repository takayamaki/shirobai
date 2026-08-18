# frozen_string_literal: true

require "spec_helper"

# `Team#each_corrector` skips a cop's corrector for the round when an
# earlier-merged cop's `autocorrect_incompatible_with` includes the cop's
# CLASS (see the core suite's autocorrect_incompatibility_spec). The core
# aligner runs at core require time, BEFORE rubocop-rails's cops exist, so
# `Rails/SafeNavigation`'s `[Style::RedundantSelf]` still named the
# dismissed stock class after this gem loaded — the skip never fired and
# shirobai's RedundantSelf corrected in a round stock drops it.
# `shirobai-rails` therefore re-runs
# `Shirobai::Inject.align_autocorrect_incompatibilities!` after its wrappers
# enlist.
RSpec.describe "autocorrect incompatibility alignment (rails)" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "leaves no replaced stock class in any enabled cop's list once the config is aligned" do
    # Non-wrapper lists are translated per config (`Inject.align_for`,
    # called from `Dispatch.bundle_token` before any correction round), not
    # at require time: enumerating the whole registry would force every
    # lazily registered stock cop to load. The runtime guarantee covers the
    # enabled set the team is built from.
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

  it "translates Rails/SafeNavigation's list to the replaced RedundantSelf" do
    Shirobai::Inject.align_for(config)
    expect(RuboCop::Cop::Rails::SafeNavigation.autocorrect_incompatible_with)
      .to eq([Shirobai::Cop::Style::RedundantSelf])
  end

  it "skips the replaced RedundantSelf after Rails/SafeNavigation corrects, like stock" do
    # Pure stock drops RedundantSelf's whole corrector in the round where
    # SafeNavigation merged (its list names the RedundantSelf class in the
    # team). The wrapper must be dropped the same way — before the aligner,
    # the list still named the dismissed stock class, the skip never fired,
    # and `self.bar` was corrected a round early. The stock pair cannot be
    # run for comparison here: the aligner has already rewritten the shared
    # `Rails/SafeNavigation` class, so the pure-stock round is pinned as an
    # absolute expectation instead.
    src = <<~RUBY
      def foo
        self.bar
        x.try!(:baz)
      end
    RUBY
    shirobai = one_team_round(
      [RuboCop::Cop::Rails::SafeNavigation, Shirobai::Cop::Style::RedundantSelf],
      src, config
    )
    expect(shirobai).to eq(<<~RUBY)
      def foo
        self.bar
        x&.baz
      end
    RUBY
  end
end
