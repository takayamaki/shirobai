# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: multi-pass `part_of_ignored_node?` accumulation.
#
# `ignore_node` accumulates ACROSS autocorrect iterations: once an outer block
# is corrected, its nested block must stay ignored on later passes (not be
# resurrected and double-corrected). shirobai feeds the accumulated correction
# ranges back to Rust which applies a `within_prior` check. This is the
# canonical instance of the §6 trap "multi-pass autocorrect state accumulation";
# if it breaks, the autocorrect diverges / double-corrects.
#
# Incidental before this spec (the non_ascii fixture happens to walk the
# cross-pass ignored-range accumulation, but it is chosen for offset coverage
# and would lose the guard if its fixture changed). This pins the accumulation
# directly: a brace multi-line block wrapping a `do..end` block, plus a separate
# single-line `do..end`. Stock and shirobai must agree on first-pass offenses
# and on the fully converged source.
RSpec.describe Shirobai::Cop::Style::BlockDelimiters do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::BlockDelimiters,
    Shirobai::Cop::Style::BlockDelimiters
  ]

  # Outer `{...}` multi-line block (-> do..end) wrapping a nested `do..end`
  # single-line block, plus a sibling single-line `do..end` (-> braces). The
  # nested block must be ignored once the outer is rewritten, so the converged
  # source is reached without re-touching it.
  it "accumulates ignored ranges across passes and converges" do
    source = "foo {\n  bar do |x| x end\n}\neach do |y| end\n"
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.size).to eq(2)
    corrected = expect_autocorrect_parity(*klasses, source, config)
    # The converged source is offence-free (no leftover nested-block offense to
    # resurrect on a further pass).
    expect(lint_offenses(klasses.last, corrected, config)).to eq(
      lint_offenses(klasses.first, corrected, config)
    )
  end

  # 1.92 behaviors probed against the stock CLI.
  describe "1.92 quirks" do
    def braces_config
      RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          { "Style/BlockDelimiters" => { "EnforcedStyle" => "always_braces" } }, "(test)"
        ),
        "(test)"
      )
    end

    it "keeps only directive comments in place (header match, any spacing)" do
      source = %w[
        #\ rubocop:disable\ Style/Foo #rubocop\ :\ todo #\ rubocop:disables
        #\ rubocop:disable-nextx #\ x\ #\ rubocop:enable #\ RuboCop:disable\ all
      ].map { |c| "foo.each { |x|\n  bar(x)\n} #{c}\n" }.join
      expect_autocorrect_parity(*klasses, source, config)
    end

    it "skips the correction only in a real loop condition" do
      source = <<~RUBY
        z while foo.any? { |x|
          x
        }
        begin
          z
        end while foo.any? { |x|
          x
        }
        while bar { foo.any? { |x|
          x
        } }
          y
        end
      RUBY
      expect_autocorrect_parity(*klasses, source, config)
    end

    it "accepts a single-line `for` collection and ignores super/yield arguments" do
      source = <<~RUBY
        for x in foo.map { |y| y }
          p x
        end
        def m
          super a, foo.map { |y|
            y
          }
          yield(foo.map { |y|
            y
          })
        end
      RUBY
      expect_autocorrect_parity(*klasses, source, config)
    end

    it "puts `end` after a heredoc that ends a rescue/ensure body" do
      source = <<~RUBY
        foo.each do |x|
          bar
        rescue StandardError
          baz(<<~EOS)
            text
          EOS
        end
        foo.each do |x|
          bar(<<~A)
            a
          A
        ensure
          q
        end
      RUBY
      expect_autocorrect_parity(*klasses, source, braces_config)
    end
  end
end
