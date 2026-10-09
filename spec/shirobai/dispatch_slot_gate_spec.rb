# frozen_string_literal: true

require "spec_helper"

# The per-slot enable gate of the shared bundle (`Shirobai::Dispatch`).
#
# `check_all` computes a core slot only when its bit is set in the slot mask
# (core nums 134 / 135, `BundleConfig::enabled_slots`). The mask holds the
# slots of the cops the config enables (`COP_KEYS` over
# `Registry#enabled`), plus the config's "forced" keys.
#
# A key is forced when a wrapper asks for a slot outside the mask. That
# happens because a wrapper can run although its cop is disabled in the
# config: stock `Team.mobilize` keeps disabled cops on a `standby_registry`
# and wakes them for a `# rubocop:enable Foo` directive
# (`Team#opted_in_standby_cops`), `--only` can name a disabled cop, and specs
# drive wrappers directly. `offenses_for` then re-runs the file with the
# wider mask, so the wrapper still sees stock's offenses.
RSpec.describe Shirobai::Dispatch do
  def config_with(hash)
    RuboCop::Config.new(RuboCop::ConfigLoader.default_configuration.to_h.merge(hash), "")
  end

  def processed(source, config)
    ps = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
    ps.config = config
    ps.registry = RuboCop::Cop::Registry.global
    ps
  end

  def lint(klass, source, config)
    cop = klass.new(config)
    report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed(source, config))
    expect(report.errors).to be_empty
    report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos, o.message] }.sort
  end

  def mask_of(nums)
    core = nums[0]
    low, high = core[134], core[135]
    (low & 0xFFFF_FFFF_FFFF_FFFF) | ((high & 0xFFFF_FFFF_FFFF_FFFF) << 64)
  end

  def bit(key)
    1 << described_class::SLOTS.fetch(key)[1]
  end

  let(:core_wrappers) do
    Shirobai::Inject.wrapper_cops.reject do |klass|
      klass.cop_name.start_with?("RSpec/", "Rails/", "Performance/")
    end
  end

  describe "COP_KEYS" do
    it "lists every core wrapper cop" do
      expect(described_class::COP_KEYS.keys).to match_array(core_wrappers.map(&:cop_name))
    end

    it "maps to every core SLOTS key at least once, and to nothing else" do
      core_keys = described_class::SLOTS.select { |_, (origin, _)| origin.zero? }.keys
      mapped = described_class::COP_KEYS.values.flatten
      expect(mapped.uniq).to match_array(core_keys)
    end

    it "matches the keys each wrapper passes to offenses_for" do
      core_wrappers.each do |klass|
        file = Object.const_source_location(klass.name)[0]
        keys = File.read(file).scan(/offenses_for\([^,]+,\s*[^,]+,\s*:([a-z_]+)\)/).flatten
        keys = [klass::SLOT.to_s] if keys.empty? && klass.const_defined?(:SLOT)
        next if keys.empty? # shared base (complexity) or not bundled

        expect(described_class::COP_KEYS.fetch(klass.cop_name).map(&:to_s)).to match_array(keys.uniq),
                                                                                  klass.cop_name
      end
    end
  end

  describe "slot mask packing" do
    it "appends the mask as two nums at core indices 134 and 135" do
      nums, = described_class.send(:packed_config, config_with({}))
      expect(nums[0].size).to eq(136)
      nums[0].last(2).each do |word|
        expect(word).to be_a(Integer)
        expect(word).to be_between(-(2**63), (2**63) - 1)
      end
    end

    it "sets exactly the bits of the enabled keys" do
      config = config_with({})
      nums, = described_class.send(:packed_config, config)
      want = described_class.enabled_keys(config).sum { |key| bit(key) }
      expect(mask_of(nums)).to eq(want)
    end

    it "carries a bit at index 63 as a negative low word" do
      # Slot 63 (EmptyLineAfterGuardClause) is the sign bit of the low word.
      expect(described_class::SLOTS.fetch(:empty_line_after_guard_clause)).to eq([0, 63])
      config = config_with({})
      nums, = described_class.send(:packed_config, config)
      expect(described_class.enabled_keys(config)).to include(:empty_line_after_guard_clause)
      expect(nums[0][134]).to be_negative
    end

    it "adds the forced keys to the mask" do
      config = config_with("Layout/ExtraSpacing" => { "Enabled" => false })
      plain, = described_class.send(:packed_config, config)
      forced, = described_class.send(:packed_config, config, [], [:extra_spacing])
      expect(mask_of(plain) & bit(:extra_spacing)).to eq(0)
      expect(mask_of(forced)).to eq(mask_of(plain) | bit(:extra_spacing))
    end

    it "turns the cop's own enable num on for a forced cop" do
      config = config_with(
        "Layout/ExtraSpacing" => { "Enabled" => false },
        "Layout/SpaceAroundOperators" => { "Enabled" => false },
        "Naming/AsciiIdentifiers" => { "Enabled" => false },
        "Layout/RescueEnsureAlignment" => { "Enabled" => false }
      )
      plain, = described_class.send(:packed_config, config)
      expect(plain[0].values_at(121, 128, 132, 133)).to eq([0, 0, 0, 0])
      keys = %i[ascii_identifiers extra_spacing rescue_ensure_alignment space_around_operators]
      forced, = described_class.send(:packed_config, config, [], keys)
      expect(forced[0].values_at(121, 128, 132, 133)).to eq([1, 1, 2, 1])
    end
  end

  describe ".enabled_keys" do
    it "holds the keys of the enabled cops only" do
      config = config_with(
        "Layout/LineLength" => { "Enabled" => false },
        "Metrics/CyclomaticComplexity" => { "Enabled" => false }
      )
      keys = described_class.enabled_keys(config)
      expect(keys).not_to include(:line_length, :line_length_breakables)
      # PerceivedComplexity is still on and shares the slot.
      expect(keys).to include(:complexity, :debugger)
    end

    it "is memoized per config identity" do
      config = config_with({})
      expect(described_class.enabled_keys(config)).to be(described_class.enabled_keys(config))
      expect(described_class.enabled_keys(config_with({}))).not_to be(described_class.enabled_keys(config))
    end
  end

  describe "forced re-run" do
    it "gives offenses to a wrapper whose cop the config disables" do
      config = config_with("Layout/ExtraSpacing" => { "Enabled" => false })
      source = "x  = 1\n"
      expect(described_class.enabled_keys(config)).not_to include(:extra_spacing)
      stock = lint(RuboCop::Cop::Layout::ExtraSpacing, source, config)
      expect(stock).not_to be_empty
      expect(lint(Shirobai::Cop::Layout::ExtraSpacing, source, config)).to eq(stock)
      expect(described_class.forced_keys(config)).to eq([:extra_spacing])
    end

    it "re-runs a file already in the cache when a forced key joins" do
      config = config_with("Layout/IndentationWidth" => { "Enabled" => false })
      ps = processed("def foo\n    bar\nend\n", config)
      # An enabled cop fills the cache first; the disabled one asks next.
      described_class.offenses_for(ps, config, :debugger)
      expect(described_class.offenses_for(ps, config, :indentation_width)).not_to be_empty
      expect(described_class.forced_keys(config)).to eq([:indentation_width])
    end

    it "leaves an off slot empty for the cops that are not asked for" do
      config = config_with("Layout/IndentationWidth" => { "Enabled" => false })
      result = Shirobai.check_all("def foo\n    bar\nend\n", described_class.bundle_token(config))
      expect(result[0][described_class::SLOTS.fetch(:indentation_width)[1]]).to be_empty
    end
  end

  describe ".bundle_token" do
    it "keys the token memo on the forced set too" do
      config = config_with("Layout/ExtraSpacing" => { "Enabled" => false })
      before = described_class.bundle_token(config)
      expect(described_class.bundle_token(config)).to eq(before)
      described_class.offenses_for(processed("x  = 1\n", config), config, :extra_spacing)
      after = described_class.bundle_token(config)
      expect(after).not_to eq(before)
      expect(described_class.bundle_token(config)).to eq(after)
    end
  end
end
