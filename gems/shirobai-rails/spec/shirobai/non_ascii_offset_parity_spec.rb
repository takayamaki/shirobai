# frozen_string_literal: true

require "spec_helper"

# Byte-vs-character offset parity for the Rails cops — same guard as the core
# suite's spec/shirobai/non_ascii_offset_parity_spec.rb (see there for the
# full rationale: Rust reports prism BYTE offsets while
# `Parser::Source::Range` indexes by CHARACTERS, so every offset field a
# wrapper receives must go through `SourceOffsets`).
#
# Each fixture puts a multibyte comment BEFORE the offense so that every byte
# offset is ahead of its char offset, and asserts first-pass offenses and the
# fully autocorrected source match stock exactly. The config force-enables the
# cop under test so Team-based autocorrect also covers `Enabled: pending` cops.
RSpec.describe "non-ASCII source offset parity with stock rubocop-rails" do
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
    "Rails/ApplicationRecord" => [
      RuboCop::Cop::Rails::ApplicationRecord,
      Shirobai::Cop::Rails::ApplicationRecord,
      "#{prefix}class Foo < ActiveRecord::Base\nend\n"
    ],
    "Rails/ApplicationRecord (Class.new)" => [
      RuboCop::Cop::Rails::ApplicationRecord,
      Shirobai::Cop::Rails::ApplicationRecord,
      "#{prefix}Foo = Class.new(::ActiveRecord::Base)\n"
    ],
    "Rails/ApplicationController" => [
      RuboCop::Cop::Rails::ApplicationController,
      Shirobai::Cop::Rails::ApplicationController,
      "#{prefix}class Foo < ActionController::Base\nend\n"
    ],
    "Rails/ApplicationMailer" => [
      RuboCop::Cop::Rails::ApplicationMailer,
      Shirobai::Cop::Rails::ApplicationMailer,
      "#{prefix}class Foo < ActionMailer::Base\nend\n"
    ],
    "Rails/ApplicationJob" => [
      RuboCop::Cop::Rails::ApplicationJob,
      Shirobai::Cop::Rails::ApplicationJob,
      "#{prefix}class Foo < ActiveJob::Base\nend\n"
    ],
    # Selector replace plus two keyword inserts, each at a multibyte-shifted
    # offset — the densest offset coverage in the cluster.
    "Rails/DynamicFindBy" => [
      RuboCop::Cop::Rails::DynamicFindBy,
      Shirobai::Cop::Rails::DynamicFindBy,
      "#{prefix}User.find_by_name_and_email(name, email)\n"
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

  # Rails/UnknownEnv has no autocorrect, so its offset parity is checked on
  # first-pass offenses only (byte-vs-char under a multibyte prefix).
  describe "Rails/UnknownEnv (no autocorrect)" do
    def offense_positions(klass, source, config)
      cop = klass.new(config)
      processed = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
      expect(report.errors).to be_empty
      report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos, o.message] }.sort
    end

    it "matches stock offense offsets on a non-ASCII source" do
      source = "#{prefix}Rails.env.proudction?\n" \
               "#{prefix}Rails.env == 'developpment'\n" \
               "case Rails.env\nwhen 'somethingg'\nend\n"
      config = config_with_enabled("Rails/UnknownEnv")
      stock = offense_positions(RuboCop::Cop::Rails::UnknownEnv, source, config)
      expect(stock).not_to be_empty
      expect(offense_positions(Shirobai::Cop::Rails::UnknownEnv, source, config)).to eq(stock)
    end
  end
end
