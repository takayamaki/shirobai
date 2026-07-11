# frozen_string_literal: true

require "spec_helper"

# `Team#each_corrector` drops a cop's whole corrector for the round when an
# earlier-merged cop's `autocorrect_incompatible_with` includes the cop's
# CLASS. Badge replacement breaks that identity check in both directions:
#
#   - a wrapper that copies stock's list verbatim still names the STOCK
#     class, which is no longer the class in the team;
#   - a stock cop that stays stock (Style/SymbolProc) lists a class shirobai
#     replaced (Layout/SpaceBeforeBlockBraces), and the wrapper instance in
#     the team never matches it.
#
# `Shirobai::Inject.align_autocorrect_incompatibilities!` rewrites every
# list through the stock-to-shirobai map after all wrappers are enlisted.
# Found via the fluentd `-a` byte audit: `Layout/SpaceInsideBlockBraces`
# lists `Style::BlockDelimiters`, so in the round where Style/Proc turns
# `Proc.new { ... }` into `proc { ... }`, stock SKIPS the BlockDelimiters
# brace-to-do/end correction and the braces survive (next round `proc {` is
# an allowed method). shirobai applied it and the trees drifted
# (fluentd types.rb / log.rb).
RSpec.describe "autocorrect incompatibility alignment" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def stock_counterpart(klass)
    dept, name = klass.cop_name.split("/")
    RuboCop::Cop.const_get(dept, false).const_get(name, false)
  rescue NameError
    nil
  end

  def wrapper_cops
    RuboCop::Cop::Registry.global.cops.select { |c| c.name&.start_with?("Shirobai::") }
  end

  it "mirrors every stock list through the badge replacement on the wrapper side" do
    replaced = wrapper_cops.to_h { |w| [stock_counterpart(w), w] }
    wrapper_cops.each do |wrapper|
      stock = stock_counterpart(wrapper)
      next unless stock

      expected = stock.autocorrect_incompatible_with.map { |k| replaced.fetch(k, k) }
      expect(wrapper.autocorrect_incompatible_with).to eq(expected),
                                                       "#{wrapper.cop_name}: expected #{expected}, " \
                                                       "got #{wrapper.autocorrect_incompatible_with}"
    end
  end

  it "leaves no dismissed stock class in any active cop's list" do
    dismissed = wrapper_cops.filter_map { |w| stock_counterpart(w) }
    RuboCop::Cop::Registry.global.cops.each do |cop|
      stale = cop.autocorrect_incompatible_with & dismissed
      expect(stale).to be_empty,
                       "#{cop.cop_name} still lists dismissed stock classes: #{stale}"
    end
  end

  it "skips BlockDelimiters in the round SpaceInsideBlockBraces corrects, like stock (fluentd types.rb)" do
    # In one round: SpaceInsideBlockBraces yields first and puts
    # BlockDelimiters on the skip list, so the multi-line `Proc.new { ... }`
    # keeps its braces this round. (In the real run Style/Proc then turns it
    # into `proc { ... }`, an allowed method, and the braces survive.)
    src = <<~RUBY
      X = Proc.new { |val|
        val
      }
      foo {puts 1}
    RUBY
    stock = one_team_round(
      [RuboCop::Cop::Layout::SpaceInsideBlockBraces, RuboCop::Cop::Style::BlockDelimiters],
      src, config
    )
    shirobai = one_team_round(
      [Shirobai::Cop::Layout::SpaceInsideBlockBraces, Shirobai::Cop::Style::BlockDelimiters],
      src, config
    )
    expect(stock).to include("Proc.new { |val|") # braces kept: BD was skipped
    expect(shirobai).to eq(stock)
  end

  it "skips the replaced SpaceBeforeBlockBraces after stock SymbolProc corrects, like stock" do
    # The other direction: stock Style/SymbolProc lists
    # Layout::SpaceBeforeBlockBraces, whose slot shirobai took over. After
    # SymbolProc merges its correction, the wrapper must be skipped exactly
    # like the stock class would be.
    src = <<~RUBY
      foo.map{ |x| x.bar }
    RUBY
    stock = one_team_round(
      [RuboCop::Cop::Style::SymbolProc, RuboCop::Cop::Layout::SpaceBeforeBlockBraces],
      src, config
    )
    shirobai = one_team_round(
      [RuboCop::Cop::Style::SymbolProc, Shirobai::Cop::Layout::SpaceBeforeBlockBraces],
      src, config
    )
    expect(stock).to include("foo.map(&:bar)") # SymbolProc won the round
    expect(shirobai).to eq(stock)
  end
end
