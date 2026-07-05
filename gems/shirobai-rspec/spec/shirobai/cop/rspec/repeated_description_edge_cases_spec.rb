# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/RepeatedDescription`.
#
# Every example runs the STOCK cop and the SHIROBAI cop side by side over the
# same source (stock fresh per file) and asserts identical offenses. Quirks
# probed against stock rubocop-rspec 3.10.2 that the vendor spec does not pin:
#
# - **C2** quote-style differences (`it 'a'` vs `it "a"`) are the same
#   structural doc string -> a repeat.
# - **C3** two zero-argument examples share the `[nil, nil]` signature, which
#   fails the `Array#any?` gate -> never a repeat.
# - **C4** different metadata (`:flag` vs none) is a distinct signature.
# - **C5** a top-level numblock example group is never checked (stock's
#   `on_block` has no numblock handler).
# - **C6** a braces (`{ }`) example group is a plain block -> checked.
# - **C7** a numblock example (`it('a') { _1 }`) is not a plain-block example.
# - **C8** a numblock context is transparent: its examples belong to the outer
#   describe and pair with siblings (offenses at BOTH nesting levels).
# - **C9** `its` are grouped by `[doc_string, example]`; the always-truthy
#   Example passes the gate, so even zero-argument `its { same body }` repeats.
# - **C10** a heredoc doc string and a same-text quoted doc string are NOT
#   structurally equal (different nodes) -> not a repeat.
# - shared groups / `include_context` blocks are not example groups.
# - an example inside a `let` body still belongs to the enclosing group.
RSpec.describe Shirobai::Cop::RSpec::RepeatedDescription do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::RepeatedDescription }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  describe "descriptions" do
    it "flags a basic repeated description (C1)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            foo
          end
          it 'a' do
            bar
          end
        end
      RUBY
    end

    it "treats quote-style differences as the same description (C2)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            foo
          end
          it "a" do
            bar
          end
        end
      RUBY
    end

    it "does not group two zero-argument examples (C3)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          it { foo }
          it { bar }
        end
      RUBY
    end

    it "keeps different metadata distinct (C4)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          it 'a' do
            foo
          end
          it 'a', :flag do
            bar
          end
        end
      RUBY
    end

    it "does not equate a heredoc doc string with a same-text quoted one (C10)" do
      expect_parity(<<~'RUBY', expect_offenses: false)
        describe 'x' do
          it "line\n" do
            foo
          end
          it(<<~S) do
            line
          S
            bar
          end
        end
      RUBY
    end
  end

  describe "its" do
    it "flags repeated its (C9)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          its(:foo) { is_expected.to eq 1 }
          its(:foo) { is_expected.to eq 1 }
        end
      RUBY
    end

    it "flags zero-argument its with the same body (C9)" do
      expect_parity(<<~RUBY)
        describe 'x' do
          its { is_expected.to eq 1 }
          its { is_expected.to eq 1 }
        end
      RUBY
    end

    it "does not flag its with the same doc but different body" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          its(:foo) { is_expected.to eq 1 }
          its(:foo) { is_expected.to eq 2 }
        end
      RUBY
    end
  end

  describe "the example-group gate" do
    it "checks a braces example group (C6)" do
      expect_parity(<<~RUBY)
        describe('x') {
          it('a') { foo }
          it('a') { bar }
        }
      RUBY
    end

    it "never checks a top-level numblock group (C5)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        RSpec.describe(Foo) {
          _1
          it('a') { foo }
          it('a') { bar }
        }
      RUBY
    end

    it "does not treat a numblock example as an example (C7)" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          it('a') { _1 }
          it('a') { _2 }
        end
      RUBY
    end

    it "does not check shared groups" do
      expect_parity(<<~RUBY, expect_offenses: false)
        shared_examples 'x' do
          it('a') { foo }
          it('a') { bar }
        end
      RUBY
    end

    it "does not check include_context blocks" do
      expect_parity(<<~RUBY, expect_offenses: false)
        include_context 'x' do
          it('a') { foo }
          it('a') { bar }
        end
      RUBY
    end
  end

  describe "scope transparency" do
    it "pairs an outer example with one inside a numblock context (C8)" do
      # Both `it 'a'` belong to the describe: offenses at both nesting levels.
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            foo
          end
          context('y') {
            _1
            it 'a' do
              bar
            end
          }
        end
      RUBY
    end

    it "pairs an example inside a let body with a sibling" do
      expect_parity(<<~RUBY)
        describe 'x' do
          it 'a' do
            foo
          end
          let(:y) do
            it 'a' do
              bar
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
    # entry point over `buffer.source` (this also keeps the NodeLocator char
    # ranges aligned with the parser AST).
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe 'x' do\r\n  it 'a' do\r\n    foo\r\n  end\r\n" \
        "  it 'a' do\r\n    bar\r\n  end\r\nend\r\n"
      )
    end
  end
end
