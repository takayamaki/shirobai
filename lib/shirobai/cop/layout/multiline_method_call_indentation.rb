# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/MultilineMethodCallIndentation`.
      #
      # Rust parses the source, finds `.`-chained method calls whose selector is
      # misindented across lines, and returns the offending range, the column
      # delta and the formatted message. Ruby supplies the flattened config
      # (style + indentation widths) and applies the realignment via
      # `AlignmentCorrector` (special-casing calls that carry a multiline block).
      class MultilineMethodCallIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Alignment
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = {
          aligned: 0,
          indented: 1,
          indented_relative_to_receiver: 2
        }.freeze

        def self.cop_name = "Layout/MultilineMethodCallIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/MultilineMethodCallIndentation")

        def validate_config
          return unless style == :aligned && cop_config["IndentationWidth"]

          raise RuboCop::ValidationError,
                "The `Layout/MultilineMethodCallIndentation` " \
                "cop only accepts an `IndentationWidth` " \
                "configuration parameter when " \
                "`EnforcedStyle` is `indented`."
        end

        def on_new_investigation
          source = processed_source.raw_source
          buffer = processed_source.buffer
          base_indent_width = config.for_cop("Layout/IndentationWidth")["Width"] || 2

          offenses = Shirobai.check_multiline_method_call_indentation(
            source, STYLE_TO_U8.fetch(style), configured_indentation_width, base_indent_width
          )

          offenses.each do |start, fin, column_delta, message, body_s, body_e, end_s, end_e|
            range = Parser::Source::Range.new(buffer, start, fin)
            add_offense(range, message: message) do |corrector|
              if end_e > end_s
                correct_with_block(corrector, range, column_delta, buffer, body_s, body_e, end_s, end_e)
              else
                RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, range, column_delta)
              end
            end
          end
        end

        private

        # Mirror of the cop's block-aware autocorrect: realign the selector line,
        # the block body and the block's `end` keyword line.
        def correct_with_block(corrector, range, column_delta, buffer, body_s, body_e, end_s, end_e)
          selector_line = buffer.line_range(range.line)
          selector_range = range_between(selector_line.begin_pos, selector_line.end_pos)
          RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, selector_range, column_delta)

          if body_e > body_s
            body = Parser::Source::Range.new(buffer, body_s, body_e)
            RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, body, column_delta)
          end

          end_kw = Parser::Source::Range.new(buffer, end_s, end_e)
          end_range = range_by_whole_lines(end_kw, include_final_newline: false)
          RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, end_range, column_delta)
        end
      end
    end
  end
end
