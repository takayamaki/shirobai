# frozen_string_literal: true

# Plumbing specs for the rspec packed-config segment and the per-file gate
# (no cop behavior here — cop specs live next to each wrapper).
RSpec.describe Shirobai::RSpec do
  def config_with(hash)
    RuboCop::Config.new(hash, "#{Dir.pwd}/.rubocop.yml")
  end

  describe ".segment" do
    it "packs the sixteen role lists from the resolved Language hash" do
      config = config_with(
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
      expect(nums).to eq([1])
      expect(lists.length).to eq(16)
      expect(lists[0]).to eq(%w[describe context])
      expect(lists[8]).to eq(%w[let let!])
      expect(lists[15]).to eq(%w[subject])
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

    it "keeps the default configuration's Language round-trippable" do
      # The suite's bundle activates rubocop-rspec, so CopHelper has merged
      # its default.yml into the default configuration.
      config = RuboCop::ConfigLoader.default_configuration
      nums, lists = described_class.segment(config)
      expect(nums).to eq([1])
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

    it "registers a bundle token whose rspec segment follows the gate" do
      config = RuboCop::ConfigLoader.default_configuration
      # No cops are wired yet -> the gate is the union over an empty cop
      # set and must be false: the token must be the dormant-rspec one and
      # check_all must accept it (wire lengths line up end to end).
      gate = Shirobai::Dispatch.plugin_gates[:rspec]
      processed_source = RuboCop::ProcessedSource.new("a = 1\n", 3.1, "x_spec.rb")
      expect(described_class::COP_CLASSES).to be_empty
      expect(gate.call(config, processed_source)).to be(false)
      token = Shirobai::Dispatch.bundle_token(config, [:rspec].freeze)
      result = Shirobai.check_all("a = 1\n", token)
      expect(result.length).to eq(Shirobai::Dispatch::ORIGINS.length)
      expect(result[2]).to eq([])
    end
  end
end
