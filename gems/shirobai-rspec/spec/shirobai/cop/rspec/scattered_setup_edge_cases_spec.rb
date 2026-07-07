# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/ScatteredSetup`.
#
# Every example runs the STOCK cop and the SHIROBAI cop side by side over the
# same source (stock fresh per file) and asserts identical offenses. Quirks
# probed against stock rubocop-rspec 3.10.2 that the vendor spec does not pin:
#
# - Two `before` hooks in one group (basic offense)
# - `around` hooks are NOT flagged (stock explicitly excludes :around)
# - Hooks inside class methods are NOT flagged (`inside_class_method?`)
# - Different scopes don't conflict (`:each` vs `:all`)
# - AC: body merge — verify the merged source matches stock byte for byte
# - Heredoc body in hook — AC respects `final_end_location`
# - CRLF fallback
RSpec.describe Shirobai::Cop::RSpec::ScatteredSetup do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::ScatteredSetup }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def expect_ac_parity(source)
    expect_autocorrect_parity(stock_class, described_class, source, config)
  end

  describe "basic detection" do
    it "flags two before hooks in one group" do
      expect_parity(<<~RUBY)
        describe Foo do
          before { bar }
          before { baz }
        end
      RUBY
    end

    it "does not flag around hooks" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          around { bar }
          around { baz }
        end
      RUBY
    end

    it "does not flag hooks inside class methods" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before { bar }
          def self.setup
            before { baz }
          end
          setup
        end
      RUBY
    end

    it "does not flag hooks inside class << self" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before { bar }
          class << self
            def setup
              before { baz }
            end
          end
        end
      RUBY
    end

    it "does not flag different scopes" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before { bar }
          before(:all) { baz }
          before(:suite) { baz }
        end
      RUBY
    end

    it "does not flag hooks in different example groups" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before { bar }

          describe '.baz' do
            before { baz }
          end
        end
      RUBY
    end

    it "does not flag hooks in different shared contexts" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          shared_context 'one' do
            before { bar }
          end

          shared_context 'two' do
            before { baz }
          end
        end
      RUBY
    end

    it "ignores similar method names inside of examples" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before { bar }

          it 'uses an instance method called before' do
            expect(before { tricky }).to_not confuse_rubocop_rspec
          end
        end
      RUBY
    end

    it "does not flag hooks with different metadata" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          before(:example) { foo }
          before(:example, :special_case) { bar }
        end
      RUBY
    end

    it "flags hooks with similar metadata" do
      expect_parity(<<~RUBY)
        describe Foo do
          before(:each, :special_case) { foo }
          before(:example, :special_case) { bar }
          before(:example, special_case: true) { bar }
          before(special_case: true) { bar }
          before(:example, special_case: false) { bar }
        end
      RUBY
    end
  end

  describe "autocorrect" do
    it "merges bodies of two before hooks" do
      source = <<~RUBY
        describe Foo do
          before { bar }
          before { baz }
        end
      RUBY
      expect_ac_parity(source)
    end

    it "merges bodies of multiple after hooks with same scope" do
      source = <<~RUBY
        describe Foo do
          after { bar }
          after(:each) { baz }
          after(:example) { baz }
        end
      RUBY
      expect_ac_parity(source)
    end

    it "handles hooks when one is an empty block" do
      source = <<~RUBY
        describe Foo do
          before { do_something }
          before { }
        end
      RUBY
      expect_ac_parity(source)
    end

    it "handles hooks with heredoc arguments" do
      source = <<~'RUBY'
        describe Foo do
          before { foo }
          before do
            bar(<<~'TEXT')
              Hello World!
            TEXT
          end
        end
      RUBY
      expect_ac_parity(source)
    end

    it "handles hooks with similar metadata" do
      source = <<~RUBY
        describe Foo do
          before(:each, :special_case) { foo }
          before(:example, :special_case) { bar }
          before(:example, special_case: true) { bar }
          before(special_case: true) { bar }
          before(:example, special_case: false) { bar }
        end
      RUBY
      expect_ac_parity(source)
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe Foo do\r\n  before { bar }\r\n  before { baz }\r\nend\r\n"
      )
    end
  end
end
