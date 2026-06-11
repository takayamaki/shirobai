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
      # `AlignmentCorrector`. Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the config derivation is purely config-driven,
      # so this cop is always bundle-eligible.
      class MultilineOperationIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        STYLE_MAP = { "aligned" => 0, "indented" => 1 }.freeze

        def self.cop_name = "Layout/MultilineOperationIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/MultilineOperationIndentation")

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
                "The `Layout/MultilineOperationIndentation` " \
                "cop only accepts an `IndentationWidth` " \
                "configuration parameter when " \
                "`EnforcedStyle` is `indented`."
        end

        def on_new_investigation
          buffer = processed_source.buffer
          offenses = Dispatch.offenses_for(processed_source, config, :multiline_operation)

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
