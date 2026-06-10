# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/MultilineOperationIndentation`.
      #
      # Rust parses the source, finds binary operations whose right-hand operand
      # is misindented, and returns the offending range together with the column
      # delta and the formatted message. Ruby supplies the flattened config
      # (style + indentation widths) and applies the realignment via
      # `AlignmentCorrector`.
      class MultilineOperationIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { aligned: 0, indented: 1 }.freeze

        def self.cop_name = "Layout/MultilineOperationIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/MultilineOperationIndentation")

        def validate_config
          return unless style == :aligned && cop_config["IndentationWidth"]

          raise RuboCop::ValidationError,
                "The `Layout/MultilineOperationIndentation` " \
                "cop only accepts an `IndentationWidth` " \
                "configuration parameter when " \
                "`EnforcedStyle` is `indented`."
        end

        def on_new_investigation
          source = processed_source.raw_source
          buffer = processed_source.buffer
          base_indent_width = config.for_cop("Layout/IndentationWidth")["Width"] || 2

          offenses = Shirobai.check_multiline_operation_indentation(
            source, STYLE_TO_U8.fetch(style), configured_indentation_width, base_indent_width
          )

          offenses.each do |start, fin, column_delta, message|
            range = Parser::Source::Range.new(buffer, start, fin)
            add_offense(range, message: message) do |corrector|
              RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, range, column_delta)
            end
          end
        end
      end
    end
  end
end
