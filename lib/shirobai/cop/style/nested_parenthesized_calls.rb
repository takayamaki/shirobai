# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/NestedParenthesizedCalls`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns the byte
      # offsets handed back into offenses and corrector ops. The corrector
      # builds an `range_with_surrounding_space(side: :left)` analogue
      # (`[ac_open_start, ac_open_end)` → `(`) plus an `insert_after` at
      # `ac_close_pos`, matching stock byte-for-byte.
      #
      # `AllowedMethods` is taken from the cop's own config (plus the deprecated
      # `IgnoredMethods` / `ExcludedMethods` aliases for parity with stock's
      # `AllowedMethods` mixin). `Regexp` entries are not supported on the
      # bundle path; if any regexp is configured, the wrapper falls back to the
      # standalone per-cop call instead.
      class NestedParenthesizedCalls < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Add parentheses to nested method call `%<source>s`."

        def self.cop_name = "Style/NestedParenthesizedCalls"
        def self.badge = RuboCop::Cop::Badge.parse("Style/NestedParenthesizedCalls")

        # Returns `[allowed_methods]`. `allowed_methods` mirrors the stock
        # `AllowedMethods` mixin: `AllowedMethods` + deprecated `IgnoredMethods`
        # + `ExcludedMethods`, with `Regexp` entries dropped (the bundle path
        # accepts string names only). When any regexp is present `bundle_args`
        # still returns the strings; the wrapper detects the regexp in
        # `bundle_eligible?` and uses the standalone path instead.
        def self.bundle_args(config)
          cfg = config.for_badge(badge)
          allowed = Array(cfg.fetch("AllowedMethods", []))
          deprecated = Array(cfg.fetch("IgnoredMethods", [])) +
                       Array(cfg.fetch("ExcludedMethods", []))
          # Stock keeps deprecated entries unless there's a Regexp in any list;
          # in that case stock returns only `AllowedMethods` (regardless of
          # type). We approximate by always concatenating both and filtering
          # Regexps out (rare in practice, and we fall back when present).
          names = (allowed + deprecated).reject { |e| e.is_a?(Regexp) }.map(&:to_s)
          [names]
        end

        def bundle_eligible?
          cfg = cop_config
          [
            Array(cfg.fetch("AllowedMethods", [])),
            Array(cfg.fetch("IgnoredMethods", [])),
            Array(cfg.fetch("ExcludedMethods", []))
          ].flatten.none? { |e| e.is_a?(Regexp) }
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses = fetch_offenses
          offenses.each do |start_offset, end_offset, ac_open_start, ac_open_end, ac_close_pos|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            message = format(MSG, source: range.source)
            add_offense(range, message: message) do |corrector|
              open_range = Parser::Source::Range.new(
                buffer, off[ac_open_start], off[ac_open_end]
              )
              close_range = Parser::Source::Range.new(
                buffer, off[ac_close_pos], off[ac_close_pos]
              )
              corrector.replace(open_range, "(")
              corrector.insert_after(close_range, ")")
            end
          end
        end

        private

        def fetch_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :nested_parenthesized_calls)
          else
            allowed = self.class.bundle_args(config)[0]
            Shirobai.check_nested_parenthesized_calls(processed_source.raw_source, allowed)
          end
        end
      end
    end
  end
end
