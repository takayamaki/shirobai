# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ClosingParenthesisIndentation`.
      #
      # Rust parses the source, walks every parenthesized method call, method
      # definition and grouped expression, and returns each hanging `)` that is
      # misindented as an offense range plus its `column_delta` and message.
      # Ruby supplies the flattened config and applies the realignment via
      # `AlignmentCorrector` over the same `)` range, exactly like stock (whose
      # `autocorrect` passes `right_paren` itself). Offenses come from the
      # per-file bundled run (`Shirobai::Dispatch`); the config is a single
      # number, so this cop is always bundle-eligible.
      class ClosingParenthesisIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/ClosingParenthesisIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/ClosingParenthesisIndentation")

        # Packed args for the bundled run: `[indentation_width]`
        # (`configured_indentation_width` from the `Alignment` mixin).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :closing_parenthesis_indentation)
          offenses.each do |start, fin, column_delta, message|
            range = Parser::Source::Range.new(buffer, start, fin)
            # Stock yields the corrector block for every offense (no per-offense
            # gating); `AlignmentCorrector` itself decides whether the corrector
            # stays empty (tabs / block comments), which keeps the lint-mode
            # `correctable?` status identical to stock.
            add_offense(range, message: message) do |corrector|
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, range, column_delta
              )
            end
          end
        end
      end
    end
  end
end
