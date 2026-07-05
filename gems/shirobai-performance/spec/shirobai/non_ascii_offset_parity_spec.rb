# frozen_string_literal: true

require "spec_helper"

# Byte-vs-character offset parity for the Performance cops — same guard as
# the core suite's spec/shirobai/non_ascii_offset_parity_spec.rb (see there
# for the full rationale: Rust reports prism BYTE offsets while
# `Parser::Source::Range` indexes by CHARACTERS, so every offset field a
# wrapper receives must go through `SourceOffsets`).
#
# Each fixture puts a multibyte comment BEFORE the offense so that every
# byte offset is ahead of its char offset, and asserts first-pass offenses
# and the fully autocorrected source match stock exactly. The config
# force-enables the cop under test so Team-based autocorrect also covers
# `Enabled: pending` cops.
RSpec.describe "non-ASCII source offset parity with stock rubocop-performance" do
  prefix = "# 多バイト文字を含むコメント\n"

  def config_with_enabled(cop_name)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash[cop_name] = (hash[cop_name] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def autocorrect_run(klass, source, config)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first_offenses = nil
    11.times do
      processed = RuboCop::ProcessedSource.new(src, RuboCop::TargetRuby::DEFAULT_VERSION)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      team = RuboCop::Cop::Team.new([cop], config, raise_error: true)
      report = team.investigate(processed)
      offenses = report.offenses.map do |o|
        [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
      end.sort
      first_offenses ||= offenses
      corrector = report.correctors.first
      break if corrector.nil? || corrector.empty?

      rewritten = corrector.rewrite
      break if rewritten == src

      src = rewritten
    end
    [first_offenses, src]
  end

  cases = {
    "Performance/Detect" => [
      RuboCop::Cop::Performance::Detect,
      Shirobai::Cop::Performance::Detect,
      "#{prefix}foo.select { |i| i.odd? }.last\n"
    ],
    "Performance/Detect (index form)" => [
      RuboCop::Cop::Performance::Detect,
      Shirobai::Cop::Performance::Detect,
      "#{prefix}foo.filter { |i| i.odd? }[0]\n"
    ],
    "Performance/StringInclude" => [
      RuboCop::Cop::Performance::StringInclude,
      Shirobai::Cop::Performance::StringInclude,
      "#{prefix}str.match?(/ab/)\n"
    ],
    "Performance/StringInclude (regexp receiver)" => [
      RuboCop::Cop::Performance::StringInclude,
      Shirobai::Cop::Performance::StringInclude,
      "#{prefix}/ab/ =~ str\n"
    ],
    "Performance/EndWith" => [
      RuboCop::Cop::Performance::EndWith,
      Shirobai::Cop::Performance::EndWith,
      "#{prefix}str.match?(/bc\\z/)\n"
    ],
    "Performance/StartWith" => [
      RuboCop::Cop::Performance::StartWith,
      Shirobai::Cop::Performance::StartWith,
      "#{prefix}/\\Aab/ =~ str\n"
    ],
    "Performance/TimesMap" => [
      RuboCop::Cop::Performance::TimesMap,
      Shirobai::Cop::Performance::TimesMap,
      "#{prefix}5.times.map { |i| i.to_s }\n"
    ]
  }

  cases.each do |name, (stock, shirobai, source)|
    it "matches stock for #{name}" do
      config = config_with_enabled(stock.cop_name)
      stock_offenses, stock_corrected = autocorrect_run(stock, source, config)
      expect(stock_offenses).not_to be_empty
      expect(stock_corrected).not_to eq(source)
      shirobai_offenses, shirobai_corrected = autocorrect_run(shirobai, source, config)
      expect(shirobai_offenses).to eq(stock_offenses)
      expect(shirobai_corrected).to eq(stock_corrected)
    end
  end
end
