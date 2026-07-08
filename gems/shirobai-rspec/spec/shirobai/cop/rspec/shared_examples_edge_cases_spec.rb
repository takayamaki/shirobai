# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/SharedExamples`.
#
# Probed against stock rubocop-rspec 3.10.2. Pins the style-independent
# candidate anchor (str/sym titles only; const and interpolated titles never
# flag) against both EnforcedStyles, plus the CRLF fallback and non-ASCII
# byte-offset paths.
RSpec.describe Shirobai::Cop::RSpec::SharedExamples do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::SharedExamples }

  def config_with(style = "string")
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    se = (hash["RSpec/SharedExamples"] || {}).dup
    hash["RSpec/SharedExamples"] = se.merge("EnforcedStyle" => style)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, config: config_with, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def expect_ac_parity(source, config: config_with)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "string style (default)" do
    it "titleizes a symbol shared example name" do
      corrected = expect_ac_parity(<<~RUBY)
        it_behaves_like :foo_bar_baz
        shared_examples :foo_bar_baz
      RUBY
      expect(corrected).to include("it_behaves_like 'foo bar baz'")
      expect(corrected).to include("shared_examples 'foo bar baz'")
    end

    it "does not flag a string title" do
      expect_parity(<<~RUBY, expect_offenses: false)
        it_behaves_like 'foo bar baz'
      RUBY
    end

    it "does not flag a const title" do
      expect_parity(<<~RUBY, expect_offenses: false)
        it_behaves_like FooBarBaz
      RUBY
    end

    it "does not flag an interpolated symbol title" do
      expect_parity(<<~'RUBY', expect_offenses: false)
        it_behaves_like :"foo_#{bar}"
      RUBY
    end
  end

  describe "symbol style" do
    it "symbolizes a string shared example name" do
      corrected = expect_ac_parity(<<~RUBY, config: config_with("symbol"))
        include_examples 'Foo Bar Baz'
        RSpec.shared_examples 'foo bar baz'
      RUBY
      expect(corrected).to include("include_examples :foo_bar_baz")
      expect(corrected).to include("RSpec.shared_examples :foo_bar_baz")
    end

    it "does not flag an interpolated string title" do
      expect_parity(<<~'RUBY', config: config_with("symbol"), expect_offenses: false)
        shared_examples "foo #{bar}"
      RUBY
    end
  end

  describe "extra arguments" do
    it "flags only the first argument (title) and keeps the rest" do
      corrected = expect_ac_parity(<<~RUBY)
        include_examples :foo_bar_baz, 'x', 'y'
      RUBY
      expect(corrected).to eq("include_examples 'foo bar baz', 'x', 'y'\n")
    end
  end

  describe "non-ASCII title" do
    it "symbolizes without shifting multibyte bytes" do
      corrected = expect_ac_parity(<<~RUBY, config: config_with("symbol"))
        it_behaves_like 'ほげ ふが'
      RUBY
      expect(corrected).to include("it_behaves_like :")
    end
  end

  describe "CRLF fallback (bundle-ineligible)" do
    it "titleizes through the standalone path" do
      source = "it_behaves_like :foo_bar\r\n"
      expect_parity(source)
      corrected = expect_ac_parity(source)
      expect(corrected).to include("it_behaves_like 'foo bar'")
    end
  end
end
