# frozen_string_literal: true

require "spec_helper"

# Lint-mode `correctable?` parity for the RSpec cops — same guard as the
# core suite's spec/shirobai/correctable_parity_spec.rb. Vendor specs always
# run with autocorrect enabled, so they never check the lint-mode status /
# `correctable?` flag. `expect_lint_parity` compares the whole snapshot tuple
# (`[begin, end, message, status, correctable?]`), so shirobai must agree with
# stock on correctability too; `expected_correctable` documents each cop.
RSpec.describe "lint-mode correctable parity with stock rubocop-rspec" do
  include EdgeCaseParity

  cases = {
    "RSpec/VariableName" => [
      RuboCop::Cop::RSpec::VariableName,
      Shirobai::Cop::RSpec::VariableName,
      "describe 'x' do\n  let(:userName) { 1 }\nend\n",
      false
    ],
    "RSpec/LetSetup" => [
      RuboCop::Cop::RSpec::LetSetup,
      Shirobai::Cop::RSpec::LetSetup,
      "describe 'x' do\n  let!(:unused) { create(:widget) }\n  it('a') { expect(1).to eq 1 }\nend\n",
      false
    ],
    "RSpec/VariableDefinition" => [
      RuboCop::Cop::RSpec::VariableDefinition,
      Shirobai::Cop::RSpec::VariableDefinition,
      "describe 'x' do\n  let('user') { 1 }\nend\n",
      true
    ],
    "RSpec/MultipleMemoizedHelpers" => [
      RuboCop::Cop::RSpec::MultipleMemoizedHelpers,
      Shirobai::Cop::RSpec::MultipleMemoizedHelpers,
      "describe 'x' do\n#{(1..6).map { |i| "  let(:v#{i}) { #{i} }\n" }.join}end\n",
      false
    ],
    "RSpec/RepeatedDescription" => [
      RuboCop::Cop::RSpec::RepeatedDescription,
      Shirobai::Cop::RSpec::RepeatedDescription,
      "describe 'x' do\n  it 'a' do\n    foo\n  end\n  it 'a' do\n    bar\n  end\nend\n",
      false
    ],
    "RSpec/RepeatedExample" => [
      RuboCop::Cop::RSpec::RepeatedExample,
      Shirobai::Cop::RSpec::RepeatedExample,
      "describe 'x' do\n  it 'a' do\n    foo\n  end\n  it 'b' do\n    foo\n  end\nend\n",
      false
    ],
    "RSpec/NamedSubject" => [
      RuboCop::Cop::RSpec::NamedSubject,
      Shirobai::Cop::RSpec::NamedSubject,
      "describe 'x' do\n  subject { described_class.new }\n  it('a') { expect(subject.foo).to be }\nend\n",
      false
    ]
  }

  cases.each do |name, (stock, shirobai, source, expected_correctable)|
    it "matches stock statuses for #{name}" do
      config = RuboCop::ConfigLoader.default_configuration
      snapshots = expect_lint_parity(stock, shirobai, source, config)
      expect(snapshots.map(&:last)).to all(be(expected_correctable))
    end
  end
end
