# frozen_string_literal: true

require "spec_helper"

# Lint-mode `correctable?` parity for the Rails cops — same guard as the core
# suite's spec/shirobai/correctable_parity_spec.rb (see there for the full
# rationale: vendor specs force the autocorrect option on, so they never check
# the lint-mode corrector/status behaviour stock reports as "[Correctable]").
RSpec.describe "lint-mode correctable parity with stock rubocop-rails" do
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
    "Rails/ApplicationRecord" => [
      RuboCop::Cop::Rails::ApplicationRecord,
      Shirobai::Cop::Rails::ApplicationRecord,
      "class Foo < ActiveRecord::Base\nend\n"
    ],
    "Rails/ApplicationRecord (Class.new)" => [
      RuboCop::Cop::Rails::ApplicationRecord,
      Shirobai::Cop::Rails::ApplicationRecord,
      "Foo = Class.new(ActiveRecord::Base)\n"
    ],
    "Rails/ApplicationController" => [
      RuboCop::Cop::Rails::ApplicationController,
      Shirobai::Cop::Rails::ApplicationController,
      "class Foo < ActionController::Base\nend\n"
    ],
    "Rails/ApplicationMailer" => [
      RuboCop::Cop::Rails::ApplicationMailer,
      Shirobai::Cop::Rails::ApplicationMailer,
      "class Foo < ActionMailer::Base\nend\n"
    ],
    "Rails/ApplicationJob" => [
      RuboCop::Cop::Rails::ApplicationJob,
      Shirobai::Cop::Rails::ApplicationJob,
      "class Foo < ActiveJob::Base\nend\n"
    ],
    "Rails/DynamicFindBy" => [
      RuboCop::Cop::Rails::DynamicFindBy,
      Shirobai::Cop::Rails::DynamicFindBy,
      "User.find_by_name(name)\n"
    ],
    "Rails/UnknownEnv" => [
      RuboCop::Cop::Rails::UnknownEnv,
      Shirobai::Cop::Rails::UnknownEnv,
      "Rails.env.proudction?\n"
    ],
    "Rails/Pluck" => [
      RuboCop::Cop::Rails::Pluck,
      Shirobai::Cop::Rails::Pluck,
      "x.map { |a| a[:foo] }\n"
    ],
    "Rails/HttpPositionalArguments" => [
      RuboCop::Cop::Rails::HttpPositionalArguments,
      Shirobai::Cop::Rails::HttpPositionalArguments,
      "get :new, user_id: 1\n"
    ],
    "Rails/DeprecatedActiveModelErrorsMethods (<< correctable)" => [
      RuboCop::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      Shirobai::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      "user.errors[:name] << 'msg'\n"
    ],
    "Rails/DeprecatedActiveModelErrorsMethods (:[]= uncorrectable)" => [
      RuboCop::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      Shirobai::Cop::Rails::DeprecatedActiveModelErrorsMethods,
      "user.errors[:name] = []\n"
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
