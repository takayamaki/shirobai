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
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # config derivation is purely config-driven, so this cop is always
      # bundle-eligible.
      class MultilineMethodCallIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Alignment
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLE_MAP = { "aligned" => 0, "indented" => 1, "indented_relative_to_receiver" => 2 }.freeze

        def self.cop_name = "Layout/MultilineMethodCallIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/MultilineMethodCallIndentation")

        # Packed args for the bundled run: `[style, indentation_width,
        # base_indentation_width]`. `EnforcedStyle` is absent when the config
        # does not mention this cop (e.g. a spec configures only the sibling
        # multiline cop, and this cop's offenses are discarded); default to the
        # first supported style (`0`) in that case.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          base = config.for_cop("Layout/IndentationWidth")["Width"] || 2
          [STYLE_MAP[cop_config["EnforcedStyle"]] || 0, cop_config["IndentationWidth"] || base, base]
        end

        def validate_config
          return unless style == :aligned && cop_config["IndentationWidth"]

          raise RuboCop::ValidationError,
                "The `Layout/MultilineMethodCallIndentation` " \
                "cop only accepts an `IndentationWidth` " \
                "configuration parameter when " \
                "`EnforcedStyle` is `indented`."
        end

        def on_new_investigation
          buffer = processed_source.buffer
          offenses = Dispatch.offenses_for(processed_source, config, :multiline_method_call)

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
