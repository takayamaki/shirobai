# frozen_string_literal: true

require "spec_helper"

# `Team#each_corrector` skips a cop's corrector for the round when an
# earlier-merged cop's `autocorrect_incompatible_with` includes the cop's
# CLASS (see the core suite's autocorrect_incompatibility_spec). The core
# aligner runs at core require time, BEFORE rubocop-rspec's cops exist, so
# `RSpec/AlignLeftLetBrace` / `RSpec/AlignRightLetBrace`'s
# `[Layout::ExtraSpacing]` still named the dismissed stock class after this
# gem loaded — the skip never fired and shirobai's ExtraSpacing corrected in
# a round stock drops it. `shirobai-rspec` therefore re-runs
# `Shirobai::Inject.align_autocorrect_incompatibilities!` after its wrappers
# enlist.
RSpec.describe "autocorrect incompatibility alignment (rspec)" do
  include EdgeCaseParity

  # Merge rubocop-rspec's defaults explicitly: RSpec/AlignLeftLetBrace needs
  # the RSpec/Language lists and the department Include, and this group does
  # not go through the vendor CopHelper integration that merges them.
  let(:config) do
    gem_path = Gem.loaded_specs.fetch("rubocop-rspec").full_gem_path
    defaults = RuboCop::ConfigLoader.load_file(File.join(gem_path, "config/default.yml"))
    base = RuboCop::ConfigLoader.default_configuration
    hash = RuboCop::ConfigLoader.merge(base.to_h, defaults.to_h)
    # AlignLeftLetBrace ships disabled; Team#roundup_relevant_cops drops
    # disabled cops before the corrector merge, so enable it for the pin.
    hash["RSpec/AlignLeftLetBrace"] = hash["RSpec/AlignLeftLetBrace"].merge("Enabled" => true)
    RuboCop::Config.new(hash, base.loaded_path)
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

  it "translates the AlignLetBrace lists to the replaced ExtraSpacing" do
    expect(RuboCop::Cop::RSpec::AlignLeftLetBrace.autocorrect_incompatible_with)
      .to eq([Shirobai::Cop::Layout::ExtraSpacing])
    expect(RuboCop::Cop::RSpec::AlignRightLetBrace.autocorrect_incompatible_with)
      .to eq([Shirobai::Cop::Layout::ExtraSpacing])
  end

  it "skips the replaced ExtraSpacing after RSpec/AlignLeftLetBrace corrects, like stock" do
    # Pure stock drops ExtraSpacing's whole corrector in the round where
    # AlignLeftLetBrace merged (its list names the ExtraSpacing class in the
    # team), so `x  = 1` survives the round. Before the aligner the list
    # still named the dismissed stock class and the wrapper corrected it a
    # round early. The stock pair cannot be run for comparison here: the
    # aligner has already rewritten the shared `RSpec/AlignLeftLetBrace`
    # class, so the pure-stock round is pinned as an absolute expectation.
    src = <<~RUBY
      let(:a) { 1 }
      let(:long) { 2 }
      x  = 1
    RUBY
    shirobai = one_team_round(
      [RuboCop::Cop::RSpec::AlignLeftLetBrace, Shirobai::Cop::Layout::ExtraSpacing],
      src, config, path: File.expand_path("spec/example_spec.rb")
    )
    expect(shirobai).to eq(<<~RUBY)
      let(:a)    { 1 }
      let(:long) { 2 }
      x  = 1
    RUBY
  end
end
