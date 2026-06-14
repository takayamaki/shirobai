# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: `begin ... end while expr` is a `while_post`,
# not a `while`, and is NOT a decision point.
#
# parser-gem represents `begin ... end while c` as `(while_post c (kwbegin ...))`,
# a distinct node from `(while c ...)`. Stock's `CyclomaticComplexity::COUNTED_NODES`
# contains `:while` / `:until` / `:for` but NOT `:while_post` / `:until_post`,
# so post-loops add nothing to either score.
#
# Prism collapses both forms into one `WhileNode` (resp. `UntilNode`) and
# distinguishes them by `is_begin_modifier()`. Without that check, shirobai
# scored every `begin ... end while` as +1 on both cyclomatic and perceived
# complexity, producing the silent +1 divergence Discourse's
# `lib/discourse_diff.rb#tokenize_markdown` and `lib/discourse_dev/config.rb#
# create_admin_user_from_input` both showed.
#
# Vendor spec coverage: vendor's CyclomaticComplexity / PerceivedComplexity
# specs cover ordinary `while` / `until` exhaustively but never a
# `begin ... end while`, so the +1 would be silent without this fixture.
RSpec.describe Shirobai::Cop::Metrics::PerceivedComplexity do
  include EdgeCaseParity

  let(:config) do
    cfg = RuboCop::ConfigLoader.default_configuration
    cfg
  end

  # `begin ... end while c`: post-loop, NOT counted.
  begin_end_while = <<~RUBY
    def m
      begin
        a
      end while c
    end
  RUBY

  # `begin ... end until c`: post-loop, NOT counted.
  begin_end_until = <<~RUBY
    def m
      begin
        a
      end until c
    end
  RUBY

  # Plain `while c do ... end`: IS counted.
  plain_while = <<~RUBY
    def m
      while c
        a
      end
    end
  RUBY

  describe Shirobai::Cop::Metrics::PerceivedComplexity do
    klasses = [
      RuboCop::Cop::Metrics::PerceivedComplexity,
      Shirobai::Cop::Metrics::PerceivedComplexity
    ]

    # `Max: 1`: a plain `while` (which counts) trips at `[2/1]`; a `begin..end
    # while` (which does NOT count) stays silent at `[1/1]`. Stock and
    # shirobai must agree.
    let(:tight_config) do
      RuboCop::Config.new(
        "Metrics/PerceivedComplexity" => {
          "Max" => 1,
          "Enabled" => true,
          "AllowedMethods" => [],
          "AllowedPatterns" => [],
          "Exclude" => []
        }
      )
    end

    it "does not count `begin ... end while`" do
      expect_lint_parity(*klasses, begin_end_while, tight_config, expect_offenses: false)
    end

    it "does not count `begin ... end until`" do
      expect_lint_parity(*klasses, begin_end_until, tight_config, expect_offenses: false)
    end

    it "still counts a plain `while`" do
      expect_lint_parity(*klasses, plain_while, tight_config)
    end
  end

  describe Shirobai::Cop::Metrics::CyclomaticComplexity do
    klasses = [
      RuboCop::Cop::Metrics::CyclomaticComplexity,
      Shirobai::Cop::Metrics::CyclomaticComplexity
    ]

    let(:tight_config) do
      RuboCop::Config.new(
        "Metrics/CyclomaticComplexity" => {
          "Max" => 1,
          "Enabled" => true,
          "AllowedMethods" => [],
          "AllowedPatterns" => [],
          "Exclude" => []
        }
      )
    end

    it "does not count `begin ... end while`" do
      expect_lint_parity(*klasses, begin_end_while, tight_config, expect_offenses: false)
    end

    it "does not count `begin ... end until`" do
      expect_lint_parity(*klasses, begin_end_until, tight_config, expect_offenses: false)
    end

    it "still counts a plain `while`" do
      expect_lint_parity(*klasses, plain_while, tight_config)
    end
  end
end
