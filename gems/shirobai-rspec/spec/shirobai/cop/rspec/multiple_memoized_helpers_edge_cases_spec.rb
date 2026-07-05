# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `RSpec/MultipleMemoizedHelpers`.
#
# Every example runs the STOCK cop and the SHIROBAI cop side by side over the
# same source (stock fresh per file) and asserts identical offenses. Quirks
# probed against stock rubocop-rspec 3.10.2 that the vendor spec does not pin:
#
# - **E2** a `let` AFTER a nested context still counts for that context
#   (verdicts are post-walk).
# - **E4/E6** a non-literal name (`let(foo)`) and an unnamed `subject { }`
#   contribute a single shared `nil` identity.
# - **E7** an arbitrary DSL block between a group and its lets is transparent
#   AND is itself an ancestor whose helpers get unioned.
# - **E8** a numblock context is transparent (its lets belong to the outer
#   group) and never gets its own offense.
# - **E9/E12** a `let` inside a `before` hook body or a `subject(:s)` body
#   counts.
# - **E11 (critical)** two `dstr`/`dsym` names differing only in interpolation
#   whitespace are structurally EQUAL, so they dedup to one — byte comparison
#   would over-count. The empty `%()` name is a parser-gem `dstr` too.
RSpec.describe Shirobai::Cop::RSpec::MultipleMemoizedHelpers do
  include EdgeCaseParity

  let(:stock_class) { RuboCop::Cop::RSpec::MultipleMemoizedHelpers }

  def config_with(max:, allow_subject: true)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["RSpec/MultipleMemoizedHelpers"] =
      (hash["RSpec/MultipleMemoizedHelpers"] || {})
      .merge("Max" => max, "AllowSubject" => allow_subject)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  def expect_parity(source, max:, allow_subject: true, expect_offenses: true)
    expect_lint_parity(stock_class, described_class, source,
                       config_with(max: max, allow_subject: allow_subject),
                       expect_offenses: expect_offenses)
  end

  describe "accumulation across scopes" do
    it "counts a later sibling let for an earlier nested context (E2)" do
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          context 'y' do
            let(:b) { 1 }
          end
          let(:a) { 1 }
        end
      RUBY
    end

    it "unions helpers through an arbitrary transparent DSL block (E7)" do
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          let(:a) { 1 }
          weird_dsl do
            let(:b) { 1 }
            context 'y' do
              let(:c) { 1 }
            end
          end
        end
      RUBY
    end

    it "attributes a numblock context's lets to the outer group (E8)" do
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          let(:a) { 1 }
          context('y') {
            _1
            let(:b) { 1 }
            let(:c) { 1 }
          }
        end
      RUBY
    end

    it "counts lets inside hook and subject bodies (E9/E12)" do
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          before do
            let(:b) { 1 }
          end
          subject(:s) do
            let(:inner) { 1 }
          end
        end
      RUBY
    end
  end

  describe "identity and uniq" do
    it "merges non-literal and unnamed-subject names into one nil (E4/E6)" do
      # subject{} (nil) + let(foo) (nil) collapse to ONE nil -> no offense.
      expect_parity(<<~RUBY, max: 1, allow_subject: false, expect_offenses: false)
        describe Foo do
          subject { 1 }
          let(foo) { 2 }
        end
      RUBY
      # nil + one real name = 2 distinct.
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          let(:a) { 1 }
          let(foo) { 2 }
        end
      RUBY
    end

    it "keeps sym and str names distinct (E5)" do
      expect_parity(<<~RUBY, max: 1)
        describe Foo do
          let(:a) { 1 }
          let('a') { 2 }
        end
      RUBY
    end
  end

  describe "dynamic (dsym/dstr) names" do
    it "dedups interpolations that differ only in whitespace (E11)" do
      # Structurally equal -> one -> no offense at Max 1.
      expect_parity(<<~'RUBY', max: 1, expect_offenses: false)
        describe Foo do
          let("a#{ b }") { 1 }
          let("a#{b}") { 2 }
        end
      RUBY
      expect_parity(<<~'RUBY', max: 1, expect_offenses: false)
        describe Foo do
          let(:"a#{ b }") { 1 }
          let(:"a#{b}") { 2 }
        end
      RUBY
    end

    it "keeps genuinely different interpolations distinct" do
      expect_parity(<<~'RUBY', max: 1)
        describe Foo do
          let("a#{b}") { 1 }
          let("c#{b}") { 2 }
        end
      RUBY
    end

    it "keeps a dsym distinct from a same-text dstr, plus a sym (E11 mixed)" do
      expect_parity(<<~'RUBY', max: 2)
        describe Foo do
          let(:"a#{x}") { 1 }
          let("a#{x}") { 2 }
          let(:c) { 3 }
        end
      RUBY
    end

    it "treats the empty %() name as a dstr" do
      # Two `%()` are structurally equal dstr nodes -> one -> no offense.
      expect_parity(<<~RUBY, max: 1, expect_offenses: false)
        describe Foo do
          let(%()) { 1 }
          let(%()) { 2 }
        end
      RUBY
    end
  end

  describe "gate" do
    it "never flags a numblock spec group even when over the limit" do
      expect_parity(<<~RUBY, max: 1)
        RSpec.describe(Foo) do
          context('y') {
            _1
            let(:a) { 1 }
            let(:b) { 1 }
          }
        end
      RUBY
    end

    it "checks shared groups (E10)" do
      expect_parity(<<~RUBY, max: 1)
        shared_examples 'x' do
          let(:a) { 1 }
          let(:b) { 1 }
        end
      RUBY
    end
  end

  describe "CRLF sources (bundle-ineligible fallback)" do
    # A CRLF source normalizes to LF in the parser buffer while `raw_source`
    # keeps the `\r`s, so shared-walk offsets no longer line up with parser
    # positions. `bundle_eligible?` must route these files to the standalone
    # entry point over `buffer.source` (this also keeps the NodeLocator
    # char ranges for dsym/dstr names aligned with the parser AST).
    it "matches stock offense positions on a CRLF source" do
      expect_parity(
        "describe 'x' do\r\n  let(:a) { 1 }\r\n  let(\"b\#{x}\") { 2 }\r\n" \
        "  let(\"b\#{y}\") { 3 }\r\nend\r\n",
        max: 2
      )
    end
  end
end
