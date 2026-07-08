# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/MultipleSubjects`.
#
# Probed against stock rubocop-rspec 3.10.2. Pins the scope-tree collection
# (barrier semantics: nested groups / shared groups isolate their own
# subjects) and the three autocorrect modes (rename to `let`, remove, skip
# `subject!`), plus the CRLF fallback and non-ASCII byte-offset paths.
RSpec.describe Shirobai::Cop::RSpec::MultipleSubjects do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::MultipleSubjects }

  def config = RuboCop::ConfigLoader.default_configuration

  def expect_parity(source, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def expect_ac_parity(source)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "named subjects" do
    it "renames all but the last to let" do
      corrected = expect_ac_parity(<<~RUBY)
        describe Foo do
          subject(:a) { 1 }
          subject(:b) { 2 }
          subject(:c) { 3 }
        end
      RUBY
      expect(corrected).to include("let(:a)")
      expect(corrected).to include("let(:b)")
      expect(corrected).to include("subject(:c)")
    end
  end

  describe "unnamed subjects" do
    it "removes all but the last" do
      corrected = expect_ac_parity(<<~RUBY)
        describe Foo do
          subject { 1 }
          subject { 2 }
          subject { 3 }
        end
      RUBY
      expect(corrected).to eq(<<~RUBY)
        describe Foo do
          subject { 3 }
        end
      RUBY
    end
  end

  describe "subject! (non-correctable)" do
    it "flags but does not correct" do
      source = <<~RUBY
        describe Foo do
          subject! { a }
          subject! { b }
        end
      RUBY
      expect_parity(source)
      corrected = expect_ac_parity(source)
      expect(corrected).to eq(source)
    end
  end

  describe "nested groups are independent scopes" do
    it "counts only subjects defined directly in each group" do
      source = <<~RUBY
        describe Foo do
          subject(:a) { 1 }
          subject(:b) { 2 }

          describe 'inner' do
            subject(:c) { 3 }
          end
        end
      RUBY
      expect_parity(source)
      corrected = expect_ac_parity(source)
      expect(corrected).to include("let(:a)")
      expect(corrected).to include("subject(:b)")
      expect(corrected).to include("subject(:c)")
    end
  end

  describe "shared example groups isolate their subjects" do
    it "does not flag subjects inside it_behaves_like blocks" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          it_behaves_like 'x' do
            subject { described_class.new(1) }
          end

          it_behaves_like 'y' do
            subject { described_class.new(2) }
          end
        end
      RUBY
    end
  end

  describe "non-ASCII subject names" do
    it "renames without shifting multibyte content" do
      corrected = expect_ac_parity(<<~RUBY)
        describe Foo do
          subject(:名前) { '日本語' }
          subject(:other) { 2 }
        end
      RUBY
      expect(corrected).to include("let(:名前)")
    end
  end

  describe "CRLF fallback (bundle-ineligible)" do
    it "flags and corrects through the standalone path" do
      source = "describe Foo do\r\n  subject(:a) { 1 }\r\n  subject(:b) { 2 }\r\nend\r\n"
      expect_parity(source)
      corrected = expect_ac_parity(source)
      expect(corrected).to include("let(:a)")
    end
  end
end
