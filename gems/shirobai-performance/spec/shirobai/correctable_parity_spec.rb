# frozen_string_literal: true

require "spec_helper"

# Lint-mode `correctable?` parity for the Performance cops — same guard as
# the core suite's spec/shirobai/correctable_parity_spec.rb (see there for
# the full rationale: vendor specs force the autocorrect option on, so they
# never check the lint-mode corrector/status behaviour stock reports as
# "[Correctable]").
RSpec.describe "lint-mode correctable parity with stock rubocop-performance" do
  def lint_offenses(klass, source)
    config = RuboCop::ConfigLoader.default_configuration
    ruby_version = RuboCop::TargetRuby::DEFAULT_VERSION
    cop = klass.new(config)
    processed = RuboCop::ProcessedSource.new(source, ruby_version)
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
    expect(report.errors).to be_empty
    report.offenses.map do |o|
      [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
    end.sort
  end

  cases = {
    "Performance/Detect" => [
      RuboCop::Cop::Performance::Detect,
      Shirobai::Cop::Performance::Detect,
      "foo.select { |i| i.odd? }.first\n"
    ],
    "Performance/Detect (index form)" => [
      RuboCop::Cop::Performance::Detect,
      Shirobai::Cop::Performance::Detect,
      "foo.filter { |i| i.odd? }[-1]\n"
    ],
    "Performance/StringInclude" => [
      RuboCop::Cop::Performance::StringInclude,
      Shirobai::Cop::Performance::StringInclude,
      "str.match?(/ab/)\n"
    ],
    "Performance/StringInclude (negation)" => [
      RuboCop::Cop::Performance::StringInclude,
      Shirobai::Cop::Performance::StringInclude,
      "str !~ /ab/\n"
    ],
    "Performance/EndWith" => [
      RuboCop::Cop::Performance::EndWith,
      Shirobai::Cop::Performance::EndWith,
      "str.match?(/bc\\z/)\n"
    ],
    "Performance/StartWith" => [
      RuboCop::Cop::Performance::StartWith,
      Shirobai::Cop::Performance::StartWith,
      "str.match?(/\\Aab/)\n"
    ],
    "Performance/TimesMap" => [
      RuboCop::Cop::Performance::TimesMap,
      Shirobai::Cop::Performance::TimesMap,
      "5.times.map { |i| i.to_s }\n"
    ]
  }

  cases.each do |name, (stock, shirobai, source)|
    it "matches stock for #{name}" do
      stock_offenses = lint_offenses(stock, source)
      expect(stock_offenses).not_to be_empty
      expect(lint_offenses(shirobai, source)).to eq(stock_offenses)
    end
  end
end
