# frozen_string_literal: true

require "spec_helper"

# Differential guard for cops that a directive opts back in.
#
# The bundle computes a core slot only for the cops the config enables
# (`Shirobai::Dispatch` slot mask). Stock RuboCop can still run a
# config-disabled cop: `Team.mobilize` keeps the registry on standby and
# wakes a disabled cop when the file has `# rubocop:enable <Cop>`
# (`Team#opted_in_standby_cops`). For such a cop the wrapper asks for a slot
# outside the mask, and `Dispatch.offenses_for` must force it on, or the
# wrapper silently reports nothing.
#
# The run goes through `Team.mobilize` with a `Registry`, the same entry the
# real CLI uses (`Runner#mobilize_team`), once with the stock classes and
# once with the shirobai classes. Both must report the same offenses.
RSpec.describe "Directive opt-in of config-disabled cops" do
  include EdgeCaseParity

  # Cops the probe cares about: the config-disabled ones, the ones with their
  # own enable num on the wire, and a few enabled neighbours.
  cop_names = %w[
    Layout/ExtraSpacing
    Layout/IndentationWidth
    Metrics/AbcSize
    Layout/SpaceAroundOperators
    Naming/AsciiIdentifiers
    Layout/RescueEnsureAlignment
    Layout/IndentationConsistency
    Layout/EndAlignment
    Lint/Debugger
  ]

  # `options` is what `Runner` passes down: `--only` arrives as
  # `{ only: [...] }` on both the registry (`Runner#mobilized_cop_classes`
  # via `filter_by_badge`) and the team (`Runner#assemble_team`).
  def cli_like_offenses(classes, source, config, options = {})
    registry = RuboCop::Cop::Registry.new(classes, options)
    team = RuboCop::Cop::Team.mobilize(registry, config, options)
    ps = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION, "probe.rb")
    ps.config = config
    ps.registry = registry
    report = team.investigate(ps)
    expect(report.errors).to be_empty
    report.offenses.map do |o|
      [o.cop_name, o.location.begin_pos, o.location.end_pos, o.message, o.status]
    end.sort
  end

  def expect_cli_parity(source, config, options = {})
    shirobai_classes = cop_names.map { |name| RuboCop::Cop::Registry.global.find_by_cop_name(name) }
    expect(shirobai_classes.map(&:name)).to all(start_with("Shirobai::"))
    stock_classes = shirobai_classes.map { |klass| Shirobai::Inject.stock_counterpart(klass) }
    stock = cli_like_offenses(stock_classes, source, config, options)
    expect(cli_like_offenses(shirobai_classes, source, config, options)).to eq(stock)
    stock
  end

  define_method(:cop_names) { cop_names }

  def config_from(yaml)
    hash = RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(YAML.safe_load(yaml), "probe/.rubocop.yml"), "probe/.rubocop.yml"
    )
    hash
  end

  let(:config) do
    config_from(<<~YAML)
      AllCops:
        NewCops: disable
        SuggestExtensions: false
      Layout/ExtraSpacing:
        Enabled: false
      Layout/IndentationWidth:
        Enabled: false
      Metrics/AbcSize:
        Enabled: false
    YAML
  end

  it "reports ExtraSpacing and IndentationWidth opted in by a directive" do
    source = <<~RUBY
      # rubocop:enable Layout/ExtraSpacing, Layout/IndentationWidth
      # frozen_string_literal: true

      x  = 1
      def foo
          bar
      end
      foo(x)
    RUBY
    stock = expect_cli_parity(source, config)
    expect(stock.map(&:first)).to include("Layout/ExtraSpacing", "Layout/IndentationWidth")
  end

  it "reports nothing for the disabled cops without the directive" do
    source = <<~RUBY
      # frozen_string_literal: true

      x  = 1
      def foo
          bar
      end
      foo(x)
    RUBY
    stock = expect_cli_parity(source, config)
    expect(stock.map(&:first)).not_to include("Layout/ExtraSpacing", "Layout/IndentationWidth")
  end

  it "reports the cops with their own enable num when a directive opts them in" do
    # SpaceAroundOperators / AsciiIdentifiers / RescueEnsureAlignment pack an
    # enable num of their own; a forced slot must turn it on as well.
    disabled = config_from(<<~YAML)
      AllCops:
        NewCops: disable
        SuggestExtensions: false
      Layout/SpaceAroundOperators:
        Enabled: false
      Naming/AsciiIdentifiers:
        Enabled: false
      Layout/RescueEnsureAlignment:
        Enabled: false
    YAML
    source = <<~RUBY
      # rubocop:enable Layout/SpaceAroundOperators, Naming/AsciiIdentifiers, Layout/RescueEnsureAlignment
      # frozen_string_literal: true

      x = 1+2
      café = x
      begin
        foo(café)
          rescue StandardError
        bar
      end
    RUBY
    stock = expect_cli_parity(source, disabled)
    expect(stock.map(&:first)).to include(
      "Layout/SpaceAroundOperators", "Naming/AsciiIdentifiers", "Layout/RescueEnsureAlignment"
    )
  end
  it "reports config-disabled cops named by --only" do
    # `Registry#enabled_cop_name?` returns true for any `--only` name, so the
    # team runs these cops although the config disables them.
    disabled = config_from(<<~YAML)
      AllCops:
        NewCops: disable
        SuggestExtensions: false
      Layout/ExtraSpacing:
        Enabled: false
      Layout/SpaceAroundOperators:
        Enabled: false
      Layout/IndentationWidth:
        Enabled: false
    YAML
    source = <<~RUBY
      # frozen_string_literal: true

      x  = 1+2
      def foo
          bar(x)
      end
    RUBY
    only = %w[Layout/ExtraSpacing Layout/SpaceAroundOperators Layout/IndentationWidth]
    stock = expect_cli_parity(source, disabled, { only: only })
    expect(stock.map(&:first).uniq).to match_array(only)
  end
end
