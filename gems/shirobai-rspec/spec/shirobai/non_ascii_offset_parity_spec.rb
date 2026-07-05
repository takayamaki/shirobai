# frozen_string_literal: true

require "spec_helper"

# Byte-vs-character offset parity for the RSpec cops — same guard as the
# core suite's spec/shirobai/non_ascii_offset_parity_spec.rb (see there for
# the full rationale: Rust reports prism BYTE offsets while
# `Parser::Source::Range` indexes by CHARACTERS, so every offset field a
# wrapper receives must go through `SourceOffsets`).
#
# Each fixture puts a multibyte comment BEFORE the offense so that every
# byte offset is ahead of its char offset. The R1 cops have no autocorrect,
# so lint-mode offense position parity is the whole check.
RSpec.describe "non-ASCII source offset parity with stock rubocop-rspec" do
  include EdgeCaseParity

  prefix = "# 多バイト文字を含むコメント\n"

  cases = {
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
    ]
  }

  cases.each do |name, (stock, shirobai, source)|
    it "keeps offense positions identical for #{name}" do
      config = RuboCop::ConfigLoader.default_configuration
      expect_lint_parity(stock, shirobai, source, config)
    end
  end
end
