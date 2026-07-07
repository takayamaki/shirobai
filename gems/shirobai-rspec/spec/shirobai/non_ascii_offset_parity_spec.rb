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
    ],
    "RSpec/PendingWithoutReason" => [
      RuboCop::Cop::RSpec::PendingWithoutReason,
      Shirobai::Cop::RSpec::PendingWithoutReason,
      "#{prefix}describe 'x' do\n  it '説明', :pending do\n  end\nend\n"
    ],
    "RSpec/Focus" => [
      RuboCop::Cop::RSpec::Focus,
      Shirobai::Cop::RSpec::Focus,
      "#{prefix}describe 'テスト', :focus do\n  it 'あ' do\n  end\nend\n"
    ],
    "RSpec/DescribedClass" => [
      RuboCop::Cop::RSpec::DescribedClass,
      Shirobai::Cop::RSpec::DescribedClass,
      "#{prefix}describe MyClass do\n  subject { MyClass.do_something }\nend\n"
    ],
    "RSpec/EmptyLineAfterExample" => [
      RuboCop::Cop::RSpec::EmptyLineAfterExample,
      Shirobai::Cop::RSpec::EmptyLineAfterExample,
      "#{prefix}describe 'x' do\n  it 'あ' do\n    値\n  end\n  it 'い' do\n    基準\n  end\nend\n"
    ],
    "RSpec/EmptyLineAfterSubject" => [
      RuboCop::Cop::RSpec::EmptyLineAfterSubject,
      Shirobai::Cop::RSpec::EmptyLineAfterSubject,
      "#{prefix}describe 'x' do\n  subject(:主体) { 値 }\n  let(:補助) { 他 }\nend\n"
    ],
    "RSpec/EmptyExampleGroup" => [
      RuboCop::Cop::RSpec::EmptyExampleGroup,
      Shirobai::Cop::RSpec::EmptyExampleGroup,
      "#{prefix}describe 'テスト' do\n  context '空のコンテキスト' do\n    let(:変数) { 値 }\n  end\nend\n"
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

  # Autocorrect parity for the metadata-family AC cops with a multibyte
  # description ahead of the metadata (every byte offset > its char offset).
  autocorrect_cases = {
    "RSpec/MetadataStyle" => [
      RuboCop::Cop::RSpec::MetadataStyle,
      Shirobai::Cop::RSpec::MetadataStyle,
      "#{prefix}describe 'テスト', a: true do\n  it 'あ' do\n  end\nend\n",
      "describe 'テスト', :a do"
    ],
    "RSpec/Focus" => [
      RuboCop::Cop::RSpec::Focus,
      Shirobai::Cop::RSpec::Focus,
      "#{prefix}describe 'テスト', :focus do\n  it 'あ' do\n  end\nend\n",
      "describe 'テスト' do"
    ],
    "RSpec/DescribedClass" => [
      RuboCop::Cop::RSpec::DescribedClass,
      Shirobai::Cop::RSpec::DescribedClass,
      "#{prefix}describe MyClass do\n  subject { MyClass.do_something }\nend\n",
      "described_class.do_something"
    ],
    "RSpec/DuplicatedMetadata" => [
      RuboCop::Cop::RSpec::DuplicatedMetadata,
      Shirobai::Cop::RSpec::DuplicatedMetadata,
      "#{prefix}describe 'テスト', :a, :a do\n  it 'あ' do\n  end\nend\n",
      "describe 'テスト', :a do"
    ],
    "RSpec/EmptyMetadata" => [
      RuboCop::Cop::RSpec::EmptyMetadata,
      Shirobai::Cop::RSpec::EmptyMetadata,
      "#{prefix}describe 'テスト', {} do\n  it 'あ' do\n  end\nend\n",
      "describe 'テスト' do"
    ],
    "RSpec/SortMetadata" => [
      RuboCop::Cop::RSpec::SortMetadata,
      Shirobai::Cop::RSpec::SortMetadata,
      "#{prefix}describe 'テスト', :b, :a do\n  it 'あ' do\n  end\nend\n",
      "describe 'テスト', :a, :b do"
    ]
  }

  autocorrect_cases.each do |name, (stock, shirobai, source, expected)|
    it "autocorrects #{name} byte-identically with a multibyte description" do
      config = RuboCop::ConfigLoader.default_configuration
      corrected = expect_autocorrect_parity(stock, shirobai, source, config)
      expect(corrected).to include(expected)
    end
  end

  it "autocorrects RSpec/EmptyLineAfterFinalLet with a multibyte offending line" do
    config = RuboCop::ConfigLoader.default_configuration
    # The offending line itself carries multibyte content, so the offense range
    # (trimmed line content) and the `"\n"` insertion point must land on the
    # correct CHARACTER offset, not the byte offset.
    source = "#{prefix}describe 'x' do\n  let(:変数) { 値 }\n  let(:別名) { 他 }\n  it 'あ' do\n    x\n  end\nend\n"
    corrected = expect_autocorrect_parity(
      RuboCop::Cop::RSpec::EmptyLineAfterFinalLet,
      Shirobai::Cop::RSpec::EmptyLineAfterFinalLet,
      source,
      config
    )
    expect(corrected).to include("let(:別名) { 他 }\n\n  it")
  end

  it "autocorrects RSpec/EmptyExampleGroup with multibyte content byte-identically" do
    config = RuboCop::ConfigLoader.default_configuration
    source = "#{prefix}describe 'テスト' do\n  context '空のコンテキスト' do\n    let(:変数) { 値 }\n  end\n\n  it '動作する' do\n    x\n  end\nend\n"
    corrected = expect_autocorrect_parity(
      RuboCop::Cop::RSpec::EmptyExampleGroup,
      Shirobai::Cop::RSpec::EmptyExampleGroup,
      source,
      config
    )
    expect(corrected).to include("it '動作する'")
    expect(corrected).not_to include("空のコンテキスト")
  end
end
