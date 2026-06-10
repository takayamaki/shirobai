# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ArgumentAlignment`.
      #
      # Rust parses the source, walks every multi-argument method call, picks the
      # alignment base for the configured `EnforcedStyle`
      # (`with_first_argument` / `with_fixed_indentation`) and returns each
      # misaligned argument as an offense range plus its `column_delta`. Ruby
      # supplies the flattened config and applies the realignment via
      # `AlignmentCorrector` (the same division of labour as the multiline
      # indentation cops).
      class ArgumentAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        ALIGN_PARAMS_MSG = "Align the arguments of a method call if they span more than one line."

        FIXED_INDENT_MSG = "Use one level of indentation for arguments " \
                           "following the first line of a multi-line method call."

        def self.cop_name = "Layout/ArgumentAlignment"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/ArgumentAlignment")

        def on_new_investigation
          buffer = processed_source.buffer
          message = fixed_indentation? ? FIXED_INDENT_MSG : ALIGN_PARAMS_MSG

          offenses_for_source.each do |start, fin, column_delta, autocorrect|
            range = Parser::Source::Range.new(buffer, start, fin)
            add_offense(range, message: message) do |corrector|
              next unless autocorrect

              RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, range, column_delta)
            end
          end
        end

        private

        def offenses_for_source
          style = fixed_indentation? ? 1 : 0
          incompatible = with_first_argument_style? && enforce_hash_argument_with_separator?
          Shirobai.check_argument_alignment(
            processed_source.raw_source, style, configured_indentation_width, incompatible
          )
        end

        def fixed_indentation?
          cop_config["EnforcedStyle"] == "with_fixed_indentation"
        end

        def with_first_argument_style?
          cop_config["EnforcedStyle"] == "with_first_argument"
        end

        def enforce_hash_argument_with_separator?
          RuboCop::Cop::Layout::HashAlignment::SEPARATOR_ALIGNMENT_STYLES.any? do |sep_style|
            config.for_enabled_cop("Layout/HashAlignment")[sep_style]&.include?("separator")
          end
        end
      end
    end
  end
end
