# frozen_string_literal: true

# Plumbing specs for the rspec packed-config segment and the per-file gate
# (no cop behavior here — cop specs live next to each wrapper).
RSpec.describe Shirobai::RSpec do
  def config_with(hash)
    RuboCop::Config.new(hash, "#{Dir.pwd}/.rubocop.yml")
  end

  describe ".segment" do
    it "packs the sixteen role lists plus the Dialect keys list from the resolved Language hash" do      config = config_with(
        "RSpec" => {
          "Language" => {
            "ExampleGroups" => {
              "Regular" => %w[describe context],
              "Focused" => %w[fdescribe],
              "Skipped" => %w[xdescribe]
            },
            "Examples" => {
              "Regular" => %w[it],
              "Focused" => %w[fit],
              "Skipped" => %w[xit],
              "Pending" => %w[pending]
            },
            "Expectations" => %w[expect],
            "Helpers" => %w[let let!],
            "Hooks" => %w[before],
            "ErrorMatchers" => %w[raise_error],
            "Includes" => {
              "Examples" => %w[include_examples],
              "Context" => %w[include_context]
            },
            "SharedGroups" => {
              "Examples" => %w[shared_examples],
              "Context" => %w[shared_context]
            },
            "Subjects" => %w[subject]
          }
        }
      )
      nums, lists = described_class.segment(config)
      expect(nums).to eq([1, 0, 0, 5, 1, 0, 1, 1, 1])
      expect(lists.length).to eq(17)
      expect(lists[0]).to eq(%w[describe context])
      expect(lists[8]).to eq(%w[let let!])
      expect(lists[15]).to eq(%w[subject])
      # 17th list: RSpec/Dialect PreferredMethods keys (none configured here).
      expect(lists[16]).to eq([])
    end

    it "packs the per-cop nums" do
      config = config_with(
        "RSpec" => { "Language" => {} },
        "RSpec/VariableName" => { "EnforcedStyle" => "camelCase" },
        "RSpec/VariableDefinition" => { "EnforcedStyle" => "strings" },
        "RSpec/MultipleMemoizedHelpers" => { "Max" => 3, "AllowSubject" => false },
        "RSpec/NamedSubject" => { "EnforcedStyle" => "named_only", "IgnoreSharedExamples" => false }
      )
      nums, = described_class.segment(config)
      # [enabled, VariableName style, VariableDefinition style, MMH Max,
      #  MMH AllowSubject, NamedSubject style, NamedSubject
      #  IgnoreSharedExamples, EmptyLineAfterExample / EmptyLineAfterHook
      #  AllowConsecutiveOneLiners]
      expect(nums).to eq([1, 1, 1, 3, 0, 1, 0, 1, 1])
    end

    it "packs the EmptyLineAfter{Example,Hook} AllowConsecutiveOneLiners nums" do
      config = config_with(
        "RSpec" => { "Language" => {} },
        "RSpec/EmptyLineAfterExample" => { "AllowConsecutiveOneLiners" => false },
        "RSpec/EmptyLineAfterHook" => { "AllowConsecutiveOneLiners" => false }
      )
      nums, = described_class.segment(config)
      expect(nums).to eq([1, 0, 0, 5, 1, 0, 1, 0, 0])
    end

    it "is memoized per config identity" do
      config = config_with("RSpec" => { "Language" => {} })
      expect(described_class.segment(config)).to equal(described_class.segment(config))
    end

    it "packs the dormant segment when the RSpec department is missing" do
      config = config_with({})
      expect(described_class.segment(config))
        .to eq(Shirobai::Dispatch::DORMANT_SEGMENTS.fetch(:rspec))
    end

    it "packs the dormant segment when Language is not a hash" do
      config = config_with("RSpec" => { "Language" => nil })
      expect(described_class.segment(config))
        .to eq(Shirobai::Dispatch::DORMANT_SEGMENTS.fetch(:rspec))
    end

    it "drops non-Array roles and non-String entries" do
      config = config_with(
        "RSpec" => {
          "Language" => {
            "Helpers" => ["let", :given, 3],
            "Subjects" => "subject"
          }
        }
      )
      _nums, lists = described_class.segment(config)
      expect(lists[8]).to eq(%w[let])
      expect(lists[15]).to eq([])
    end

    it "packs the RSpec/Dialect PreferredMethods keys into the 17th list" do
      config = config_with(
        "RSpec" => { "Language" => {} },
        "RSpec/Dialect" => {
          "PreferredMethods" => { "context" => "describe", "feature" => "describe" }
        }
      )
      _nums, lists = described_class.segment(config)
      expect(lists.length).to eq(17)
      expect(lists[16]).to contain_exactly("context", "feature")
    end

    it "keeps the default configuration's Language round-trippable" do
      # The suite's bundle activates rubocop-rspec, so CopHelper has merged
      # its default.yml into the default configuration.
      config = RuboCop::ConfigLoader.default_configuration
      nums, lists = described_class.segment(config)
      expect(nums).to eq([1, 0, 0, 5, 1, 0, 1, 1, 1])
      expect(lists[0]).to include("describe", "context", "feature", "example_group")
      expect(lists[3]).to include("it", "specify", "its")
      expect(lists[8]).to eq(%w[let let!])
    end
  end

  describe "the registered packer and gate" do
    it "is registered for the :rspec origin" do
      expect(Shirobai::Dispatch.plugin_packers[:rspec]).not_to be_nil
      expect(Shirobai::Dispatch.plugin_gates[:rspec]).not_to be_nil
    end

    it "follows the wrapper cops' relevant_file? exactly" do
      # The default configuration carries the RSpec department Include
      # (**/*_spec.rb, **/spec/**/*) and factory Excludes; the gate must
      # answer exactly like the wrappers Team would consult.
      config = RuboCop::ConfigLoader.default_configuration
      gate = Shirobai::Dispatch.plugin_gates[:rspec]
      # Exclude entries are absolutized against the config's base dir
      # (stock semantics), so the factories case must live under it.
      base = config.base_dir_for_path_parameters
      {
        "/proj/spec/models/user_spec.rb" => true,
        "/proj/spec/support/helpers.rb" => true,
        "/proj/app/models/user.rb" => false,
        "#{base}/spec/factories/users.rb" => false,
        RuboCop::AST::ProcessedSource::STRING_SOURCE_NAME => true
      }.each do |path, expected|
        processed_source = RuboCop::ProcessedSource.new("a = 1\n", 3.1, path)
        expect(gate.call(config, processed_source)).to(
          eq(expected), "gate(#{path}) should be #{expected}"
        )
        wrappers = described_class::COP_CLASSES.map { |k| k.new(config) }
        expect(wrappers.any? { |cop| cop.relevant_file?(path) }).to eq(expected)
      end
    end

    it "registers a dormant token for gated-off files that check_all accepts" do
      config = RuboCop::ConfigLoader.default_configuration
      token = Shirobai::Dispatch.bundle_token(config, [:rspec].freeze)
      result = Shirobai.check_all("describe('x') { let(:badName) { 1 } }\n", token)
      expect(result.length).to eq(Shirobai::Dispatch::ORIGINS.length)
      # Dormant segment: the RSpec slots exist but stay empty.
      expect(result[2][0]).to eq([[], []])
      expect(result[2][1]).to eq([])
      expect(result[2][2]).to eq([])
      expect(result[2][3]).to eq([])
    end

    it "computes RSpec results through the awake token" do
      config = RuboCop::ConfigLoader.default_configuration
      token = Shirobai::Dispatch.bundle_token(config)
      result = Shirobai.check_all(
        "describe('x') { let(:badName) { 1 } }\n", token
      )
      offenses, passing = result[2][0]
      expect(offenses.length).to eq(1)
      expect(offenses[0][2]).to eq(0)
      expect(offenses[0][3]).to eq("badName")
      expect(passing).to eq([])
    end
  end

  describe "the standalone fallback safety net" do
    it "returns nil from offenses_for on a gated-off file and the wrapper still matches stock" do
      config = RuboCop::ConfigLoader.default_configuration
      source = "describe('x') { let(:badName) { 1 } }\n"
      path = "/proj/app/models/user.rb"
      processed_source = RuboCop::ProcessedSource.new(source, 3.1, path)
      processed_source.config = config
      processed_source.registry = RuboCop::Cop::Registry.global
      expect(
        Shirobai::Dispatch.offenses_for(processed_source, config, :rspec_variable_name)
      ).to be_nil

      # Driving the wrapper directly (a Commissioner bypasses Team's
      # relevant_file? filter, like an editor integration might): the
      # fallback path must produce exactly the stock offenses.
      snapshots = [RuboCop::Cop::RSpec::VariableName, Shirobai::Cop::RSpec::VariableName].map do |klass|
        cop = klass.new(config)
        report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed_source)
        expect(report.errors).to be_empty
        report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos, o.message] }
      end
      expect(snapshots[0]).not_to be_empty
      expect(snapshots[1]).to eq(snapshots[0])
    end
  end
end
