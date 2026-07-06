# frozen_string_literal: true

require "spec_helper"

# Byte-vs-character offset parity for the RSpec cops — same guard as the core
# suite's spec/shirobai/non_ascii_offset_parity_spec.rb (Rust reports prism
# BYTE offsets while `Parser::Source::Range` indexes by CHARACTERS, so every
# offset field a wrapper receives must go through `SourceOffsets`).
#
# Each fixture puts a multibyte comment BEFORE the offense so that every byte
# offset is ahead of its char offset. Lint-only cops (VariableName, LetSetup,
# MultipleMemoizedHelpers) check offense-position parity; VariableDefinition
# additionally runs autocorrect to convergence with a multibyte variable name,
# asserting the corrected bytes match stock exactly.
RSpec.describe "non-ASCII source offset parity with stock rubocop-rspec" do
  include EdgeCaseParity

  prefix = "# 多バイト文字を含むコメント\n"

  lint_cases = {
    "RSpec/VariableName" => [
      RuboCop::Cop::RSpec::VariableName,
      Shirobai::Cop::RSpec::VariableName,
      "#{prefix}describe 'x' do\n  let(:userName) { 1 }\nend\n"
    ],
    "RSpec/VariableName (multibyte name)" => [
      RuboCop::Cop::RSpec::VariableName,
      Shirobai::Cop::RSpec::VariableName,
      "#{prefix}describe 'x' do\n  let(:ユーザ名) { 1 }\nend\n"
    ],
    "RSpec/LetSetup" => [
      RuboCop::Cop::RSpec::LetSetup,
      Shirobai::Cop::RSpec::LetSetup,
      "#{prefix}describe 'x' do\n  let!(:未使用) { create(:widget) }\n  it('a') { expect(1).to eq 1 }\nend\n"
    ],
    "RSpec/MultipleMemoizedHelpers" => [
      RuboCop::Cop::RSpec::MultipleMemoizedHelpers,
      Shirobai::Cop::RSpec::MultipleMemoizedHelpers,
      "#{prefix}describe 'x' do\n#{(1..6).map { |i| "  let(:変数#{i}) { #{i} }\n" }.join}end\n"
    ],
    "RSpec/RepeatedDescription" => [
      RuboCop::Cop::RSpec::RepeatedDescription,
      Shirobai::Cop::RSpec::RepeatedDescription,
      "#{prefix}describe 'x' do\n  it '説明' do\n    foo\n  end\n  it '説明' do\n    bar\n  end\nend\n"
    ],
    "RSpec/RepeatedExample" => [
      RuboCop::Cop::RSpec::RepeatedExample,
      Shirobai::Cop::RSpec::RepeatedExample,
      "#{prefix}describe 'x' do\n  it 'あ' do\n    expect(値).to be(基準)\n  end\n  it 'い' do\n    expect(値).to be(基準)\n  end\nend\n"
    ],
    "RSpec/NamedSubject" => [
      RuboCop::Cop::RSpec::NamedSubject,
      Shirobai::Cop::RSpec::NamedSubject,
      "#{prefix}describe 'x' do\n  subject { described_class.new }\n  it('検証') { expect(subject.値).to be }\nend\n"
    ]
  }

  lint_cases.each do |name, (stock, shirobai, source)|
    it "keeps offense positions identical for #{name}" do
      config = RuboCop::ConfigLoader.default_configuration
      expect_lint_parity(stock, shirobai, source, config)
    end
  end

  it "autocorrects RSpec/VariableDefinition with a multibyte name byte-identically" do
    config = RuboCop::ConfigLoader.default_configuration
    # Multibyte comment ahead of a multibyte string name; symbols style turns
    # `let("ユーザ")` into `let(:ユーザ)`.
    source = "#{prefix}describe 'x' do\n  let(\"ユーザ\") { 1 }\nend\n"
    corrected = expect_autocorrect_parity(
      RuboCop::Cop::RSpec::VariableDefinition,
      Shirobai::Cop::RSpec::VariableDefinition,
      source,
      config
    )
    expect(corrected).to include("let(:ユーザ)")
  end
end
