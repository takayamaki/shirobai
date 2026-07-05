# frozen_string_literal: true

require "spec_helper"

# Lint-mode `correctable?` parity for the RSpec cops — same guard as the
# core suite's spec/shirobai/correctable_parity_spec.rb. The R1 cops carry
# no autocorrect, so the check pins that shirobai reports them exactly as
# uncorrectable as stock does (status / correctable? are part of the
# snapshot tuple).
RSpec.describe "lint-mode correctable parity with stock rubocop-rspec" do
  include EdgeCaseParity

  cases = {
    "RSpec/VariableName" => [
      RuboCop::Cop::RSpec::VariableName,
      Shirobai::Cop::RSpec::VariableName,
      "describe 'x' do\n  let(:userName) { 1 }\nend\n"
    ],
    "RSpec/LetSetup" => [
      RuboCop::Cop::RSpec::LetSetup,
      Shirobai::Cop::RSpec::LetSetup,
      "describe 'x' do\n  let!(:unused) { create(:widget) }\n  it('a') { expect(1).to eq 1 }\nend\n"
    ]
  }

  cases.each do |name, (stock, shirobai, source)|
    it "matches stock statuses for #{name}" do
      config = RuboCop::ConfigLoader.default_configuration
      snapshots = expect_lint_parity(stock, shirobai, source, config)
      expect(snapshots.map(&:last)).to all(be(false))
    end
  end
end
