# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/LetSetup`.
#
# Quirks probed against stock rubocop-rspec 3.10.2 that the vendor spec
# does not pin:
#
# - **Zero-argument uses only**: stock's `method_called?` pattern is
#   `(send nil? %)` with no argument wildcard, so `w(1)` and `w(&b)` are
#   NOT uses (still flagged) while `w` and `w { }` are.
# - **Names never cross kinds**: `let!('w')` and `let!(:w)` do not shadow
#   each other in the override check, but a use resolves both (stock
#   symbolizes the name before searching).
# - **Only exact `let_bang` shapes**: `let!(:w, :extra) { }`, dstr names
#   and numblock `let!` blocks are not candidates.
# - **numblock groups are transparent**: `context('y') { _1; let!(:w)... }`
#   attributes the `let!` to the outer describe (scope_change? is a
#   `(block ...)` pattern), and a use anywhere in the outer subtree
#   resolves it.
# - **Self-reference counts**: `let!(:w) { w }` finds its own body's `w`.
# - **Language-configurable collection**: removing `let!` from
#   `RSpec/Language` Helpers stops the collection (no offenses); adding a
#   `given!` alias collects it but the literal `let_bang` pattern still
#   never matches it.
RSpec.describe Shirobai::Cop::RSpec::LetSetup do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::LetSetup }
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def expect_parity(source, config: self.config, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source, config,
                       expect_offenses: expect_offenses)
  end

  def config_with_helpers(helpers)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    rspec = (hash["RSpec"] || {}).dup
    lang = (rspec["Language"] || {}).dup
    lang["Helpers"] = helpers
    rspec["Language"] = lang
    hash["RSpec"] = rspec
    RuboCop::Config.new(hash, default.loaded_path)
  end

  describe "zero-argument use resolution" do
    it "still flags when the name is only called with arguments" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let!(:w) { create(:widget) }
          it('a') { expect(w(1)).to be }
        end
      RUBY
    end

    it "still flags when the name is only called with a block-pass" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let!(:w) { create(:widget) }
          it('a') { w(&b) }
        end
      RUBY
    end

    it "resolves a use with a literal block" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          let!(:w) { create(:widget) }
          it('a') { w { 1 } }
        end
      RUBY
    end

    it "resolves a self-referencing let! body" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          let!(:w) { w }
          it('a') { expect(1).to eq 1 }
        end
      RUBY
    end

    it "does not count local variable references as uses" do
      # `w = 1; w` parses as lvasgn + lvar, not a send, on both parsers.
      expect_parity(<<~RUBY)
        describe 'x' do
          let!(:w) { create(:widget) }
          it('a') do
            w = 1
            expect(w).to eq 1
          end
        end
      RUBY
    end
  end

  describe "name kinds" do
    it "resolves a string-named let! by symbolized use" do
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          let!('w') { create(:widget) }
          it('a') { expect(w).to be }
        end
      RUBY
    end

    it "does not let sym and str names shadow each other" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let!('w') { create(:widget) }
          context 'y' do
            let!(:w) { create(:other) }
            it('a') { expect(1).to eq 1 }
          end
        end
      RUBY
    end

    it "skips extra-argument, dstr-named and numblock let! forms" do
      expect_parity(<<~'RUBY', expect_offenses: false)
        describe 'x' do
          let!(:w, :extra) { create(:widget) }
          let!("w#{x}") { create(:widget) }
          let!(:v) { _1 }
          it('a') { expect(1).to eq 1 }
        end
      RUBY
    end

    it "flags the block-pass send form" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let!(:w, &blk)
          it('a') { expect(1).to eq 1 }
        end
      RUBY
    end
  end

  describe "scopes and overrides" do
    it "flags the outer let! and skips the overriding inner one" do
      expect_parity(<<~RUBY)
        describe 'x' do
          let!(:w) { create(:widget) }
          context 'y' do
            let!(:w) { create(:other) }
            it('a') { expect(1).to eq 1 }
          end
        end
      RUBY
    end

    it "treats numblock groups as transparent scopes" do
      expect_parity(<<~RUBY)
        describe 'x' do
          context('y') {
            _1
            let!(:w) { create(:widget) }
          }
          it('a') { expect(1).to eq 1 }
        end
      RUBY
      expect_parity(<<~RUBY, expect_offenses: false)
        describe 'x' do
          context('y') {
            _1
            let!(:w) { create(:widget) }
          }
          it('a') { expect(w).to be }
        end
      RUBY
    end

    it "flags inside include_context blocks" do
      expect_parity(<<~RUBY)
        include_context 'shared' do
          let!(:w) { create(:widget) }
        end
      RUBY
    end

    it "keeps the no-parentheses send range" do
      offenses = expect_parity(<<~RUBY)
        describe 'x' do
          let! :w do
            create(:widget)
          end
          it('a') { expect(1).to eq 1 }
        end
      RUBY
      # The offense covers `let! :w` only (the parser send node).
      expect(offenses.first[1] - offenses.first[0]).to eq("let! :w".length)
    end
  end

  describe "RSpec/Language configuration" do
    it "goes silent when let! is removed from Helpers" do
      expect_parity(<<~RUBY, config: config_with_helpers(%w[let]), expect_offenses: false)
        describe 'x' do
          let!(:w) { create(:widget) }
          it('a') { expect(1).to eq 1 }
        end
      RUBY
    end

    it "never matches aliases even when configured as Helpers" do
      expect_parity(
        <<~RUBY, config: config_with_helpers(%w[let let! given!]), expect_offenses: false
          describe 'x' do
            given!(:w) { create(:widget) }
            it('a') { expect(1).to eq 1 }
          end
        RUBY
      )
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source`.
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe 'x' do\r\n  let!(:w) { create(:widget) }\r\n" \
        "  it('a') { expect(1).to eq 1 }\r\nend\r\n"
      )
    end
  end
end
