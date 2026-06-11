# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/DotPosition`.
      #
      # Rust finds dots (`.`/`&.`) that violate the enforced multi-line position
      # and returns the offense range together with the autocorrect ranges
      # (what to remove, where to re-insert the dot). Ruby supplies the style and
      # applies the corrections. Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the style is purely config-driven, so this cop
      # is always bundle-eligible.
      class DotPosition < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { "leading" => 0, "trailing" => 1 }.freeze

        def self.cop_name = "Layout/DotPosition"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/DotPosition")

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::RedundantSelf]
        end

        # Packed args for the bundled run: `[style]`. `EnforcedStyle` may be
        # absent when the config does not mention this cop (the slice is then
        # discarded); default to `leading` (0) in that case.
        def self.bundle_args(config)
          [STYLE_TO_U8[config.for_badge(badge)["EnforcedStyle"]] || 0]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          Dispatch.offenses_for(processed_source, config, :dot_position)
                  .each do |dot_start, dot_end, remove_start, remove_end, insert_pos|
            dot = Parser::Source::Range.new(buffer, dot_start, dot_end)
            add_offense(dot, message: message(dot)) do |corrector|
              corrector.remove(Parser::Source::Range.new(buffer, remove_start, remove_end))
              corrector.insert_before(Parser::Source::Range.new(buffer, insert_pos, insert_pos), dot.source)
            end
          end
        end

        private

        def message(dot)
          "Place the #{dot.source} on the " +
            case style
            when :leading
              "next line, together with the method name."
            when :trailing
              "previous line, together with the method call receiver."
            end
        end
      end
    end
  end
end
