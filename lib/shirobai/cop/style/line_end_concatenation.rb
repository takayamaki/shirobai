# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/LineEndConcatenation`.
      #
      # Rust detects string literal concatenations broken across lines with `+`
      # or `<<` and computes the autocorrection range. Ruby reports the offense
      # and replaces the operator (plus trailing whitespace) with `\`. Offenses
      # come from the per-file bundled run (`Shirobai::Dispatch`); the cop has
      # no configuration, so it is always bundle-eligible.
      class LineEndConcatenation < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Use `\\` instead of `%<operator>s` to concatenate multiline strings."

        def self.cop_name = "Style/LineEndConcatenation"
        def self.badge = RuboCop::Cop::Badge.parse("Style/LineEndConcatenation")

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::RedundantInterpolation]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :line_end_concatenation)
          offenses.each do |op_start, op_end, operator, replace_start, replace_end|
            range = Parser::Source::Range.new(buffer, op_start, op_end)
            message = format(MSG, operator: operator)
            add_offense(range, message: message) do |corrector|
              replace_range = Parser::Source::Range.new(buffer, replace_start, replace_end)
              corrector.replace(replace_range, "\\")
            end
          end
        end
      end
    end
  end
end
