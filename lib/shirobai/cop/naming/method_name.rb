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
      # When none of those filters is configured, Rust only returns the
      # invalid sites (see `on_new_investigation`).
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
          # Fast path: with no AllowedPatterns / ForbiddenIdentifiers /
          # ForbiddenPatterns configured (the default), the style-compliant
          # sites can only ever feed `correct_style_detected`, so Rust drops
          # them and reports their existence as a flag instead. `style_detected`
          # is idempotent set-intersection bookkeeping (sticky
          # `no_acceptable_style!`), so one call is observably identical to one
          # call per compliant site, in any order relative to
          # `unexpected_style_detected`.
          fast = allowed_patterns.empty? && forbidden_identifiers.empty? && forbidden_patterns.empty?

          candidates, had_valid = Shirobai.check_method_name(
            processed_source.raw_source, STYLE_INDEX.fetch(style.to_s), fast
          )
          correct_style_detected if fast && had_valid

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
