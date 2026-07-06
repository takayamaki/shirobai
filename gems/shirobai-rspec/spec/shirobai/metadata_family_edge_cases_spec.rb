# frozen_string_literal: true

require "spec_helper"

# Differential edge-case parity for the R2 metadata-family cops. Every example
# runs the SAME snippet through the stock rubocop-rspec cop and the shirobai
# wrapper and asserts identical offenses (and autocorrected bytes for the AC
# cops). These pin the quirks probed against stock 3.10.2 — corpus positives
# for this family are thin, so this synthetic coverage is the real guard.
RSpec.describe "metadata-family edge-case parity with stock rubocop-rspec" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  # --- block-kind: metadata cops fire on PLAIN blocks only (numblock never;
  # itblock == plain at target < 3.4). Shirobai emits every block kind and the
  # wrapper's parser matcher self-filters, so it tracks stock exactly. ---

  describe "block kind" do
    it "matches stock: numblock metadata does not fire (MetadataStyle)" do
      source = "RSpec.describe 'x' do\n  it('y', a: true) { _1 }\nend\n"
      expect_autocorrect_parity(
        RuboCop::Cop::RSpec::MetadataStyle,
        Shirobai::Cop::RSpec::MetadataStyle, source, config
      )
    end

    it "matches stock: plain inner block metadata fires (MetadataStyle)" do
      source = "RSpec.describe 'x' do\n  it('y', a: true) { foo }\nend\n"
      corrected = expect_autocorrect_parity(
        RuboCop::Cop::RSpec::MetadataStyle,
        Shirobai::Cop::RSpec::MetadataStyle, source, config
      )
      expect(corrected).to include("it('y', :a)")
    end

    it "matches stock: numblock duplicated metadata does not fire" do
      source = "RSpec.describe 'x' do\n  it('y', :a, :a) { _1 }\nend\n"
      expect_autocorrect_parity(
        RuboCop::Cop::RSpec::DuplicatedMetadata,
        Shirobai::Cop::RSpec::DuplicatedMetadata, source, config
      )
    end
  end

  # --- Focus quirks ---

  describe "Focus" do
    {
      "focus metadata symbol removal" =>
        "describe 'x', :focus do\n  it 'a' do\n  end\nend\n",
      "focus:true pair removal" =>
        "RSpec.describe MyClass, focus: true do\nend\n",
      "fdescribe rename" => "fdescribe 'c' do\nend\n",
      "fit rename" => "fit 'b' do\nend\n",
      "bare focus alias is not correctable" => "focus 'd' do\nend\n",
      "focused numblock still fires (send-based)" => "fit('b') { _1 }\n",
      "xit with focus metadata (skipped inside focusable)" =>
        "describe 'x' do\n  xit 'f', :focus do\n  end\nend\n"
    }.each do |name, source|
      it "matches stock: #{name}" do
        expect_autocorrect_parity(
          RuboCop::Cop::RSpec::Focus, Shirobai::Cop::RSpec::Focus, source, config
        )
      end
    end

    {
      "chained receiver does not fire" => "foo.fdescribe 'x' do\nend\n",
      "inside a def does not fire" =>
        "describe 'x' do\n  def helper\n    fit('a') { }\n  end\nend\n",
      "non-rspec receiver does not fire" => "Foo.fit 'a' do\nend\n"
    }.each do |name, source|
      it "matches stock (no offense): #{name}" do
        expect_lint_parity(
          RuboCop::Cop::RSpec::Focus, Shirobai::Cop::RSpec::Focus, source, config,
          expect_offenses: false
        )
      end
    end
  end

  # --- PendingWithoutReason quirks (no autocorrect) ---

  describe "PendingWithoutReason" do
    {
      "pending block form" => "describe 'x' do\n  pending 'p' do\n  end\nend\n",
      "pending metadata symbol" => "describe 'x' do\n  it 'a', :pending do\n  end\nend\n",
      "bare pending in example body" =>
        "describe 'x' do\n  it 'b' do\n    pending\n  end\nend\n",
      "xdescribe example group" => "describe 'x' do\n  xdescribe 'c' do\n  end\nend\n",
      "skip block form" => "describe 'x' do\n  skip 'd' do\n  end\nend\n",
      "bare skip in example body" =>
        "describe 'x' do\n  it 'f' do\n    skip\n  end\nend\n",
      "xit block form" => "describe 'x' do\n  xit 'j' do\n  end\nend\n"
    }.each do |name, source|
      it "matches stock: #{name}" do
        expect_lint_parity(
          RuboCop::Cop::RSpec::PendingWithoutReason,
          Shirobai::Cop::RSpec::PendingWithoutReason, source, config
        )
      end
    end

    {
      "regular example without body does not fire" =>
        "describe 'x' do\n  it 'g'\nend\n",
      "pending with reason string does not fire" =>
        "describe 'x' do\n  it 'h' do\n    pending 'reason'\n  end\nend\n",
      "pending: reason pair does not fire" =>
        "describe 'x' do\n  it 'i', pending: 'reason' do\n  end\nend\n"
    }.each do |name, source|
      it "matches stock (no offense): #{name}" do
        expect_lint_parity(
          RuboCop::Cop::RSpec::PendingWithoutReason,
          Shirobai::Cop::RSpec::PendingWithoutReason, source, config,
          expect_offenses: false
        )
      end
    end
  end

  # --- configure path + hooks + shared groups feed the shared anchor list ---

  describe "metadata anchor sources" do
    it "matches stock: RSpec.configure hook metadata (MetadataStyle)" do
      source = "RSpec.configure do |c|\n  c.before(:each, a: true) { foo }\nend\n"
      corrected = expect_autocorrect_parity(
        RuboCop::Cop::RSpec::MetadataStyle,
        Shirobai::Cop::RSpec::MetadataStyle, source, config
      )
      expect(corrected).to include("c.before(:each, :a)")
    end

    it "matches stock: hook + shared group empty metadata removal" do
      source = "describe 'x' do\n  before(:each, {}) { foo }\nend\n" \
               "shared_examples 'y', {} do\n  it { foo }\nend\n"
      expect_autocorrect_parity(
        RuboCop::Cop::RSpec::EmptyMetadata,
        Shirobai::Cop::RSpec::EmptyMetadata, source, config
      )
    end

    it "matches stock: SortMetadata trailing symbols with mixed args" do
      source = "describe 'x' do\n  context 'e', :z, variable, :a, :b do\n  end\nend\n"
      expect_autocorrect_parity(
        RuboCop::Cop::RSpec::SortMetadata,
        Shirobai::Cop::RSpec::SortMetadata, source, config
      )
    end
  end
end
