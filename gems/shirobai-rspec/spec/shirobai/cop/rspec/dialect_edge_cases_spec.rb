# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/Dialect`.
#
# Probed against stock rubocop-rspec 3.10.2. These pin the Rust candidate
# narrowing (configured `PreferredMethods` keys only) against stock's
# `rspec_method?` + `preferred_methods` behavior, including the CRLF fallback
# (bundle-ineligible) and non-ASCII byte-offset paths.
RSpec.describe Shirobai::Cop::RSpec::Dialect do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::Dialect }

  def config_with(preferred = { "context" => "describe" })
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    d = (hash["RSpec/Dialect"] || {}).dup
    # `Enabled: true` because EdgeCaseParity's autocorrect path runs through a
    # Team, which skips cops disabled in config (Dialect defaults to disabled).
    hash["RSpec/Dialect"] = d.merge("Enabled" => true, "PreferredMethods" => preferred)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, config: config_with, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def expect_ac_parity(source, config: config_with)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "configured alias" do
    it "flags and corrects a bare context block" do
      corrected = expect_ac_parity(<<~RUBY)
        context 'display name presence' do
        end
      RUBY
      expect(corrected).to include("describe 'display name presence'")
    end

    it "flags and corrects an RSpec.context block (explicit receiver)" do
      corrected = expect_ac_parity(<<~RUBY)
        RSpec.context 'thing' do
          it('works') { expect(1).to eq(1) }
        end
      RUBY
      expect(corrected).to include("RSpec.describe 'thing'")
    end

    it "does not flag a context call with a non-rspec receiver" do
      expect_parity(<<~RUBY, expect_offenses: false)
        it 'reads context' do
          expect(request.context).to be_empty
        end
      RUBY
    end
  end

  describe "empty PreferredMethods keeps every dialect method" do
    it "does not flag anything" do
      expect_parity(<<~RUBY, config: config_with({}), expect_offenses: false)
        context 'is important' do
          specify('leeway') { everyone.should have_some_leeway }
        end
      RUBY
    end
  end

  describe "ErrorMatchers alias" do
    it "flags and corrects raise_exception" do
      config = config_with("raise_exception" => "raise_error")
      corrected = expect_ac_parity(<<~RUBY, config: config)
        it 'raises' do
          expect { subject }.to raise_exception(StandardError)
        end
      RUBY
      expect(corrected).to include("raise_error(StandardError)")
    end
  end

  describe "multiple configured keys" do
    it "flags each configured alias in one pass" do
      config = config_with("context" => "describe", "feature" => "describe")
      corrected = expect_ac_parity(<<~RUBY, config: config)
        context 'a' do
        end
        feature 'b' do
        end
      RUBY
      expect(corrected).to include("describe 'a'")
      expect(corrected).to include("describe 'b'")
    end
  end

  describe "non-ASCII byte offsets" do
    it "corrects the selector without shifting the multibyte title" do
      corrected = expect_ac_parity(<<~RUBY)
        context 'ユーザー名の存在' do
          it('あ') { expect(1).to eq(1) }
        end
      RUBY
      expect(corrected).to include("describe 'ユーザー名の存在'")
    end
  end

  describe "CRLF fallback (bundle-ineligible)" do
    it "flags and corrects through the standalone path" do
      source = "context 'x' do\r\nend\r\n"
      expect_parity(source)
      corrected = expect_ac_parity(source)
      expect(corrected).to include("describe 'x'")
    end
  end
end
