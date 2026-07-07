# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/DescribedClass`.
#
# Probed against stock rubocop-rspec 3.10.2.
RSpec.describe Shirobai::Cop::RSpec::DescribedClass do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::DescribedClass }

  def config_with(overrides = {})
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    dc = (hash["RSpec/DescribedClass"] || {}).dup
    hash["RSpec/DescribedClass"] = dc.merge(overrides)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, config: config_with, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def expect_ac_parity(source, config: config_with)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "described_class style (default)" do
    context "basic const replacement" do
      it "flags and corrects a const matching the describe argument" do
        source = <<~RUBY
          describe MyClass do
            subject { MyClass.do_something }
          end
        RUBY
        corrected = expect_ac_parity(source)
        expect(corrected).to include("described_class.do_something")
      end

      it "does not flag when the const does not match" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            subject { OtherClass.do_something }
          end
        RUBY
      end
    end

    context "OnlyStaticConstants: true (default)" do
      it "does not flag Const::CONSTANT" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            subject { MyClass::CONSTANT }
          end
        RUBY
      end
    end

    context "OnlyStaticConstants: false" do
      it "flags Const::CONSTANT" do
        config = config_with("OnlyStaticConstants" => false)
        source = <<~RUBY
          describe MyClass do
            subject { MyClass::CONSTANT }
          end
        RUBY
        expect_parity(source, config: config)
      end
    end

    context "SkipBlocks: true" do
      it "skips non-RSpec blocks but flags RSpec blocks" do
        config = config_with("SkipBlocks" => true)
        source = <<~RUBY
          describe MyClass do
            controller(ApplicationController) do
              self.bar = MyClass
            end

            before do
              MyClass
            end
          end
        RUBY
        expect_parity(source, config: config)
        corrected = expect_ac_parity(source, config: config)
        expect(corrected).to include("self.bar = MyClass")
        expect(corrected).to include("described_class\n")
      end
    end

    context "scope changes stop traversal" do
      it "does not flag inside def" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            def some_method
              MyClass
            end
          end
        RUBY
      end

      it "does not flag inside inner class" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            class Inner
              MyClass
            end
          end
        RUBY
      end

      it "does not flag inside module" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            module Inner
              MyClass
            end
          end
        RUBY
      end

      it "does not flag inside common_instance_exec_closure" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            Class.new do
              MyClass
            end
          end
        RUBY
      end
    end

    context "namespace resolution" do
      it "resolves nested modules" do
        source = <<~RUBY
          module A
            describe B do
              subject { B.new }
            end
          end
        RUBY
        corrected = expect_ac_parity(source)
        expect(corrected).to include("described_class.new")
      end

      it "handles cbase const (::MyClass)" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            subject { ::MyClass.new }
          end
        RUBY
      end
    end

    context "described_class inside a const path" do
      it "does not flag described_class::CONSTANT usage" do
        expect_parity(<<~RUBY, expect_offenses: false)
          describe MyClass do
            subject { described_class::CONSTANT }
          end
        RUBY
      end
    end
  end

  describe "explicit style" do
    it "flags described_class and replaces with const name" do
      config = config_with("EnforcedStyle" => "explicit")
      source = <<~RUBY
        describe MyClass do
          subject { described_class.do_something }
        end
      RUBY
      corrected = expect_ac_parity(source, config: config)
      expect(corrected).to include("MyClass.do_something")
    end
  end

  describe "autocorrect byte-for-byte parity" do
    it "matches stock correction for described_class style" do
      source = <<~RUBY
        describe MyClass do
          subject { MyClass.do_something }
          before { MyClass.setup }
        end
      RUBY
      expect_ac_parity(source)
    end

    it "matches stock correction for explicit style" do
      config = config_with("EnforcedStyle" => "explicit")
      source = <<~RUBY
        describe MyClass do
          subject { described_class.do_something }
          before { described_class.setup }
        end
      RUBY
      expect_ac_parity(source, config: config)
    end
  end

  describe "CRLF fallback" do
    it "handles CRLF line endings via fallback path" do
      source = "describe MyClass do\r\n  subject { MyClass.do_something }\r\nend\r\n"
      expect_parity(source)
    end
  end
end
