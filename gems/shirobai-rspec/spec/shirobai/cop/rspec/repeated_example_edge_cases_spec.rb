# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/RepeatedExample`.
#
# Every example runs the STOCK cop and the SHIROBAI cop side by side over the
# same source (stock fresh per file) and asserts identical offenses. Quirks
# probed against stock rubocop-rspec 3.10.2 that the vendor spec does not pin:
#
# - **D1** the same body with different descriptions is a repeat; the message
#   lists the OTHER examples' first lines (`.uniq.sort`, comma-joined).
# - **D2** two empty-braces bodies (`it('a') { }` / `it('b') { }`) are a repeat
#   (metadata `[]` + implementation `nil` are equal); there is no `Array#any?`
#   gate on this cop.
# - **D3** quote-style (`eq('v')` vs `eq("v")`) and paren (`eq 1` vs `eq(1)`)
#   differences are the same AST -> a repeat.
# - **D4** distinct metadata is not a repeat: `:flag` vs none, and `:flag` vs
#   `flag: true` (a sym is not a pair).
# - **D5** a `"line\n"` string body and a `<<~S` heredoc body are NOT
#   structurally equal -> not a repeat.
# - shared groups / `include_context` blocks are not example groups.
# - an example inside a `let` body still belongs to the enclosing group.
RSpec.describe Shirobai::Cop::RSpec::RepeatedExample do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::RepeatedExample }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  describe "repeated implementations" do
    it "flags the same body under different descriptions (D1)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it "does x" do
            expect(foo).to be(bar)
          end
          it "does y" do
            expect(foo).to be(bar)
          end
        end
      RUBY
    end

    it "flags empty-braces bodies (D2)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it('a') { }
          it('b') { }
        end
      RUBY
    end

    it "treats quote-style body differences as a repeat (D3)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            expect(x).to eq('v')
          end
          it 'b' do
            expect(x).to eq("v")
          end
        end
      RUBY
    end

    it "treats paren-style body differences as a repeat (D3)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            expect(x).to eq 1
          end
          it 'b' do
            expect(x).to eq(1)
          end
        end
      RUBY
    end

    it "does not equate a string body with a same-text heredoc body (D5)" do
      expect_parity(<<~'RUBY', expect_offenses: false)
        describe 'x' do
          it 'a' do
            foo("line\n")
          end
          it 'b' do
            foo(<<~S)
              line
            S
          end
        end
      RUBY
    end
  end

  describe "metadata (D4)" do
    it "does not flag a body repeated with distinct metadata" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          it 'a' do
            expect(foo).to be(bar)
          end
          it 'b', :flag do
            expect(foo).to be(bar)
          end
        end
      RUBY
    end

    it "does not equate a sym flag with a keyword flag" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          it 'a', :flag do
            foo
          end
          it 'b', flag: true do
            foo
          end
        end
      RUBY
    end
  end

  describe "its" do
    it "flags repeated its with the same args and body" do
      expect_parity(<<~RUBY)
        describe 'x' do
          its(:foo) { is_expected.to eq 1 }
          its(:foo) { is_expected.to eq 1 }
          its(:bar) { is_expected.to eq 1 }
        end
      RUBY
    end
  end

  describe "the example-group gate" do
    it "never checks a top-level numblock group" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe(Foo) {
          _1
          it('a') { foo }
          it('a') { foo }
        }
      RUBY
    end

    it "does not check shared groups" do
      expect_parity(<<~RUBY, expect_offenses: false)
        shared_examples 'x' do
          it('a') { foo }
          it('a') { foo }
        end
      RUBY
    end

    it "does not check include_context blocks" do
      expect_parity(<<~RUBY, expect_offenses: false)
        include_context 'x' do
          it('a') { foo }
          it('a') { foo }
        end
      RUBY
    end
  end

  describe "scope transparency" do
    it "pairs an outer example with one inside a numblock context" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            expect(foo).to be(bar)
          end
          context('y') {
            _1
            it 'b' do
              expect(foo).to be(bar)
            end
          }
        end
      RUBY
    end

    it "pairs an example inside a let body with a sibling" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            expect(foo).to be(bar)
          end
          let(:y) do
            it 'b' do
              expect(foo).to be(bar)
            end
          end
        end
      RUBY
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source`.
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe 'x' do\r\n  it 'a' do\r\n    expect(foo).to be(bar)\r\n  end\r\n" \
        "  it 'b' do\r\n    expect(foo).to be(bar)\r\n  end\r\nend\r\n"
      )
    end
  end
end
