# frozen_string_literal: true

module Shirobai
  module Cop
    module Naming
      # Drop-in Rust reimplementation of `Naming/MethodName`.
      #
      # Rust walks the method-name sites (`def`/`defs`, `define_method`,
      # `Struct.new`/`Data.define` members, `alias`/`alias_method` arguments and
      # `attr_*` accessors), filters out operator methods, computes whether each
      # name matches the configured `EnforcedStyle` (and which alternative style
      # it matches otherwise), including the class-emitter-method exception, and
      # returns the offense candidates. Ruby keeps the `AllowedPatterns`,
      # `ForbiddenIdentifiers` and `ForbiddenPatterns` filters and the
      # `ConfigurableEnforcedStyle` bookkeeping (`config_to_allow_offenses`).
      class MethodName < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableNaming
        include RuboCop::Cop::AllowedPattern
        include RuboCop::Cop::ForbiddenIdentifiers
        include RuboCop::Cop::ForbiddenPattern

        MSG = "Use %<style>s for method names."
        MSG_FORBIDDEN = "`%<identifier>s` is forbidden, use another method name instead."

        STYLE_INDEX = { "snake_case" => 0, "camelCase" => 1 }.freeze
        INDEX_STYLE = %i[snake_case camelCase].freeze

        def self.cop_name = "Naming/MethodName"
        def self.badge = RuboCop::Cop::Badge.parse("Naming/MethodName")

        def on_new_investigation
          candidates = Shirobai.check_method_name(
            processed_source.raw_source, STYLE_INDEX.fetch(style.to_s)
          )

          candidates.each do |start, fin, name, valid, alt, fb_start, fb_end, fb_name|
            next if matches_allowed_pattern?(name)

            if forbidden_name?(name)
              fb_range = Parser::Source::Range.new(processed_source.buffer, fb_start, fb_end)
              add_offense(fb_range, message: format(MSG_FORBIDDEN, identifier: fb_name))
            elsif valid
              correct_style_detected
            else
              range = Parser::Source::Range.new(processed_source.buffer, start, fin)
              add_offense(range, message: format(MSG, style: style)) do
                if alt == 255
                  unrecognized_style_detected
                else
                  unexpected_style_detected(INDEX_STYLE[alt])
                end
              end
            end
          end
        end

        private

        def forbidden_name?(name)
          forbidden_identifier?(name) || forbidden_pattern?(name)
        end
      end
    end
  end
end
