# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/ParenthesesAsGroupedExpression`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns the byte
      # offsets handed back into offenses and a single `corrector.remove(range)`
      # call (matching stock's `add_offense(range) { |c| c.remove(range) }`).
      #
      # The cop carries no config (stock has neither `EnforcedStyle` nor
      # `AllowedMethods`), so `bundle_args` returns an empty vector and the
      # bundle path is always taken.
      class ParenthesesAsGroupedExpression < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "`%<argument>s` interpreted as grouped expression."

        def self.cop_name = "Lint/ParenthesesAsGroupedExpression"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/ParenthesesAsGroupedExpression")

        # Config-less cop. Returns an empty array so `Dispatch.packed_config`
        # can splat it without touching `nums` or `lists`.
        def self.bundle_args(_config)
          []
        end

        def bundle_eligible?
          true
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          fetch_offenses.each do |space_start, space_end, arg_start, arg_end|
            range = Parser::Source::Range.new(buffer, off[space_start], off[space_end])
            arg_range = Parser::Source::Range.new(buffer, off[arg_start], off[arg_end])
            message = format(MSG, argument: arg_range.source)
            add_offense(range, message: message) do |corrector|
              corrector.remove(range)
            end
          end
        end

        private

        def fetch_offenses
          Dispatch.offenses_for(processed_source, config, :parentheses_as_grouped_expression)
        end
      end
    end
  end
end
