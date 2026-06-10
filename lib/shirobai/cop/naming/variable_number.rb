# frozen_string_literal: true

module Shirobai
  module Cop
    module Naming
      # Drop-in Rust reimplementation of `Naming/VariableNumber`.
      #
      # Rust walks the identifiers (parameters, variable assignments, method
      # names, symbols), applies the numbering check, and the `AllowedIdentifiers`
      # filter, returning only the offenders plus whether the configured style
      # was used correctly anywhere. Ruby keeps the `AllowedPatterns` filter and
      # the `ConfigurableEnforcedStyle` bookkeeping (`config_to_allow_offenses`).
      class VariableNumber < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableNumbering
        include RuboCop::Cop::AllowedIdentifiers
        include RuboCop::Cop::AllowedPattern

        MSG = "Use %<style>s for %<identifier_type>s numbers."

        STYLE_INDEX = { "snake_case" => 0, "normalcase" => 1, "non_integer" => 2 }.freeze
        INDEX_STYLE = %i[snake_case normalcase non_integer].freeze
        TYPE_LABEL = ["variable", "method name", "symbol"].freeze

        def self.cop_name = "Naming/VariableNumber"
        def self.badge = RuboCop::Cop::Badge.parse("Naming/VariableNumber")

        def on_new_investigation
          flags = (cop_config["CheckMethodNames"] ? 2 : 0) | (cop_config["CheckSymbols"] ? 1 : 0)
          offenses, had_correct = Shirobai.check_variable_number(
            processed_source.raw_source, STYLE_INDEX.fetch(style.to_s), flags, allowed_identifiers
          )

          saw_correct = had_correct
          offenses.each do |start, fin, id_type, name, alt|
            # A name the Rust side flagged may still be exempt by AllowedPatterns,
            # in which case it counts as a correct use of the configured style.
            if matches_allowed_pattern?(name)
              saw_correct = true
              next
            end

            range = Parser::Source::Range.new(processed_source.buffer, start, fin)
            message = format(MSG, style: style, identifier_type: TYPE_LABEL[id_type])
            add_offense(range, message: message) do
              if alt == 255
                unrecognized_style_detected
              else
                unexpected_style_detected(INDEX_STYLE[alt])
              end
            end
          end

          correct_style_detected if saw_correct
        end
      end
    end
  end
end
