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
    ],
    # Focus corrects the metadata-removal form (and the fdescribe rename).
    "RSpec/Focus" => [
      RuboCop::Cop::RSpec::Focus,
      Shirobai::Cop::RSpec::Focus,
      "describe 'x', :focus do\n  it 'a' do\n  end\nend\n",
      true
    ],
    # PendingWithoutReason has no autocorrect.
    "RSpec/PendingWithoutReason" => [
      RuboCop::Cop::RSpec::PendingWithoutReason,
      Shirobai::Cop::RSpec::PendingWithoutReason,
      "describe 'x' do\n  it 'a', :pending do\n  end\nend\n",
      false
    ],
    "RSpec/MetadataStyle" => [
      RuboCop::Cop::RSpec::MetadataStyle,
      Shirobai::Cop::RSpec::MetadataStyle,
      "describe 'x', a: true do\n  it 'y' do\n  end\nend\n",
      true
    ],
    "RSpec/DuplicatedMetadata" => [
      RuboCop::Cop::RSpec::DuplicatedMetadata,
      Shirobai::Cop::RSpec::DuplicatedMetadata,
      "describe 'x', :a, :a do\n  it 'y' do\n  end\nend\n",
      true
    ],
    "RSpec/EmptyMetadata" => [
      RuboCop::Cop::RSpec::EmptyMetadata,
      Shirobai::Cop::RSpec::EmptyMetadata,
      "describe 'x', {} do\n  it 'y' do\n  end\nend\n",
      true
    ],
    "RSpec/SortMetadata" => [
      RuboCop::Cop::RSpec::SortMetadata,
      Shirobai::Cop::RSpec::SortMetadata,
      "describe 'x', :b, :a do\n  it 'y' do\n  end\nend\n",
      true
    ],
    "RSpec/EmptyLineAfterExample" => [
      RuboCop::Cop::RSpec::EmptyLineAfterExample,
      Shirobai::Cop::RSpec::EmptyLineAfterExample,
      "describe 'x' do\n  it 'a' do\n    foo\n  end\n  it 'b' do\n    bar\n  end\nend\n",
      true
    ],
    "RSpec/EmptyLineAfterExampleGroup" => [
      RuboCop::Cop::RSpec::EmptyLineAfterExampleGroup,
      Shirobai::Cop::RSpec::EmptyLineAfterExampleGroup,
      "describe 'x' do\n  context 'a' do\n    foo\n  end\n  context 'b' do\n    bar\n  end\nend\n",
      true
    ],
    "RSpec/EmptyLineAfterFinalLet" => [
      RuboCop::Cop::RSpec::EmptyLineAfterFinalLet,
      Shirobai::Cop::RSpec::EmptyLineAfterFinalLet,
      "describe 'x' do\n  let(:a) { 1 }\n  let(:b) { 2 }\n  it 'x' do\n    y\n  end\nend\n",
      true
    ],
    "RSpec/EmptyLineAfterHook" => [
      RuboCop::Cop::RSpec::EmptyLineAfterHook,
      Shirobai::Cop::RSpec::EmptyLineAfterHook,
      "describe 'x' do\n  before do\n    a\n  end\n  it 'x' do\n    y\n  end\nend\n",
      true
    ],
    "RSpec/EmptyLineAfterSubject" => [
      RuboCop::Cop::RSpec::EmptyLineAfterSubject,
      Shirobai::Cop::RSpec::EmptyLineAfterSubject,
      "describe 'x' do\n  subject(:obj) { described_class }\n  let(:foo) { bar }\nend\n",
      true
    ],
    "RSpec/EmptyExampleGroup" => [
      RuboCop::Cop::RSpec::EmptyExampleGroup,
      Shirobai::Cop::RSpec::EmptyExampleGroup,
      "describe Foo do\n  context 'empty' do\n    let(:foo) { bar }\n  end\nend\n",
      true
    ],
    "RSpec/DescribedClass" => [
      RuboCop::Cop::RSpec::DescribedClass,
      Shirobai::Cop::RSpec::DescribedClass,
      "describe MyClass do\n  subject { MyClass.do_something }\nend\n",
      true
    ],
    # ScatteredSetup: the first occurrence in each repeated group has
    # correctable?=false (stock's autocorrect returns early for
    # first_occurrence == occurrence, so no corrector calls are made and
    # RuboCop marks it :unsupported). Only the second+ occurrences are
    # correctable. We skip the global `all(be(expected_correctable))`
    # assertion and check the mixed statuses directly below.
    "RSpec/ScatteredSetup" => [
      RuboCop::Cop::RSpec::ScatteredSetup,
      Shirobai::Cop::RSpec::ScatteredSetup,
      "describe 'x' do\n  before { bar }\n  before { baz }\nend\n",
      nil
    ]
  }

  cases.each do |name, (stock, shirobai, source, expected_correctable)|
    it "matches stock statuses for #{name}" do
      config = RuboCop::ConfigLoader.default_configuration
      snapshots = expect_lint_parity(stock, shirobai, source, config)
      # When expected_correctable is nil the cop has mixed per-offense
      # correctability (e.g. ScatteredSetup's first occurrence is not
      # correctable). The parity assertion already verified agreement;
      # skip the uniform assertion.
      next if expected_correctable.nil?

      expect(snapshots.map(&:last)).to all(be(expected_correctable))
    end
  end
end
