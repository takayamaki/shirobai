# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/FirstArgumentIndentation`.
      #
      # Rust parses the source, walks every method call with arguments, decides
      # the alignment base for the configured `EnforcedStyle`, and returns the
      # first argument as an offense range plus its `column_delta`, the message,
      # the `within?` autocorrect flag and the range to realign (which for
      # `special_for_inner_method_call_in_parentheses` may be the whole receiver
      # chain). Ruby supplies the flattened config and applies the realignment
      # via `AlignmentCorrector` (the same division of labour as the other
      # alignment cops).
      class FirstArgumentIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "special_for_inner_method_call_in_parentheses" => 0,
          "consistent" => 1,
          "consistent_relative_to_receiver" => 2,
          "special_for_inner_method_call" => 3
        }.freeze

        def self.cop_name = "Layout/FirstArgumentIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/FirstArgumentIndentation")

        def on_new_investigation
          buffer = processed_source.buffer

          offenses_for_source.each do |start, fin, column_delta, message, autocorrect, cs, ce|
            range = Parser::Source::Range.new(buffer, start, fin)
            # Key the split on the per-offense flag, not `autocorrect?` mode: the
            # block runs in lint mode too and the non-empty corrector is what
            # keeps the offense correctable to match stock (see argument_alignment).
            unless autocorrect
              add_offense(range, message: message)
              next
            end

            add_offense(range, message: message) do |corrector|
              correct_range = Parser::Source::Range.new(buffer, cs, ce)
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, correct_range, column_delta
              )
            end
          end
        end

        private

        def offenses_for_source
          Shirobai.check_first_argument_indentation(
            processed_source.raw_source,
            STYLES.fetch(style.to_s, 0),
            configured_indentation_width,
            enforce_fixed_with_no_line_break?
          )
        end

        def enforce_fixed_with_no_line_break?
          enforce_first_argument_with_fixed_indentation? &&
            !enable_layout_first_method_argument_line_break?
        end

        def enforce_first_argument_with_fixed_indentation?
          argument_alignment_config = config.for_enabled_cop("Layout/ArgumentAlignment")
          argument_alignment_config["EnforcedStyle"] == "with_fixed_indentation"
        end

        def enable_layout_first_method_argument_line_break?
          config.cop_enabled?("Layout/FirstMethodArgumentLineBreak")
        end
      end
    end
  end
end
