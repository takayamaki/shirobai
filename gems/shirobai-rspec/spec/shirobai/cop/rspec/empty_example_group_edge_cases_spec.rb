# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/EmptyExampleGroup`.
#
# Every example runs the STOCK cop and the SHIROBAI cop side by side over the
# same source (stock fresh per file) and asserts identical offenses and
# autocorrect output. Quirks probed against stock rubocop-rspec 3.10.2 that
# the vendor spec does not pin:
#
# - **E1** basic empty context inside a describe is flagged (the basic case).
# - **E2** context with examples inside a non-hook block is NOT flagged
#   (`examples_inside_block?` matches a non-hook block).
# - **E3** context with examples in conditional branches is NOT flagged
#   (`examples_in_branches?` matches).
# - **E4** inside a `def` method is NOT flagged
#   (`each_ancestor(:any_def)` guard).
# - **E5** `inside_example?` guard prevents flagging.
# - **E6** autocorrect: removed range matches stock byte for byte
#   (whole lines, including final newline).
# - **E7** CRLF fallback produces the same offense positions.
RSpec.describe Shirobai::Cop::RSpec::EmptyExampleGroup do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::EmptyExampleGroup }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  describe "basic detection" do
    it "flags an empty context inside a describe (E1)" do
      expect_parity(<<~RUBY)
        describe Foo do
          context 'when bar' do
            let(:foo) { bar }
          end

          it 'something' do
            expect(true).to be(true)
          end
        end
      RUBY
    end

    it "flags an empty top-level describe" do
      expect_parity(<<~RUBY)
        describe Foo do
        end
      RUBY
    end

    it "does not flag a describe with a nil body (no block contents)" do
      # `describe Foo` without a block is not a block node.
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          it 'a' do
            expect(1).to eq(1)
          end
        end
      RUBY
    end
  end

  describe "non-hook block with examples (E2)" do
    it "does not flag context with examples inside a custom block" do
      expect_parity(<<~RUBY, expect_offenses: false)
        context 'with custom block' do
          mute_warnings do
            it { expect(1).to eq(1) }
          end
        end
      RUBY
    end

    it "does not flag context with examples inside an iterator block" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'iteration' do
          [1, 2, 3].each do |n|
            it { expect(n).to be > 0 }
          end
        end
      RUBY
    end
  end

  describe "conditional branches with examples (E3)" do
    it "does not flag context with examples in if branches" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'conditionals' do
          if RUBY_VERSION >= '2.3'
            it { expect(true).to be(true) }
          else
            warn 'old ruby'
          end
        end
      RUBY
    end

    it "does not flag context with examples in case branches" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          case bar
          when baz
            it { expect(result).to be(true) }
          end
        end
      RUBY
    end

    it "flags when conditional branches have no examples" do
      expect_parity(<<~RUBY)
        describe Foo do
          if condition
            warn 'a'
          else
            warn 'b'
          end
        end
      RUBY
    end
  end

  describe "inside a def method (E4)" do
    it "does not flag example groups inside a def" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe Foo do
          def self.with_feature(&block)
            context 'with feature' do
              module_exec(&block)
            end
          end

          with_feature do
            it_behaves_like 'feature'
          end
        end
      RUBY
    end

    it "does not flag example groups inside a class << self def" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe Foo do
          class << self
            def without_feature(&block)
              context 'without feature' do
                module_exec(&block)
              end
            end
          end

          without_feature do
            it_behaves_like 'feature'
          end
        end
      RUBY
    end
  end

  describe "inside_example? guard (E5)" do
    it "does not flag example groups inside examples" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe 'rspec-core' do
          it 'runs an example group' do
            group = RSpec.describe { }
            group.run
          end
        end
      RUBY
    end
  end

  describe "autocorrect parity (E6)" do
    it "removes the whole empty context lines byte for byte" do
      source = <<~RUBY
        describe Foo do
          context 'empty' do
            let(:foo) { bar }
          end

          it 'something' do
            expect(true).to be(true)
          end
        end
      RUBY
      expect_autocorrect_parity(stock_class, described_class, source, config)
    end

    it "removes the entire file when the top-level describe is empty" do
      source = <<~RUBY
        describe Foo do
        end
      RUBY
      expect_autocorrect_parity(stock_class, described_class, source, config)
    end

    it "removes multiple empty groups in the same file" do
      source = <<~RUBY
        context 'hook with implicit scope' do
          before do
            it { is_expected.to never_run }
          end
        end

        context 'hook with explicit scope' do
          around(:example) do
            it { is_expected.to never_run }
          end
        end
      RUBY
      expect_autocorrect_parity(stock_class, described_class, source, config)
    end
  end

  describe "CRLF fallback (E7)" do
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe Foo do\r\n  context 'empty' do\r\n    let(:foo) { bar }\r\n  end\r\n\r\n" \
        "  it 'something' do\r\n    expect(true).to be(true)\r\n  end\r\nend\r\n"
      )
    end
  end

  describe "include methods" do
    it "does not flag when include_examples is present" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          context 'with include' do
            include_examples 'shared stuff'
          end
        end
      RUBY
    end

    it "does not flag when it_behaves_like is present" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          context 'with it_behaves_like' do
            it_behaves_like 'shared stuff'
          end
        end
      RUBY
    end

    it "does not flag pending examples" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe Foo do
          it 'will be implemented later'
        end
      RUBY
    end
  end
end
