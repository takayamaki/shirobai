# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/TrailingCommaInArguments`.
      #
      # Rust walks every method call once, replicating stock's `on_send` /
      # `on_csend` and the shared `TrailingComma#check` mixin: for a
      # parenthesized call (or an index `[]` call) with arguments it decides
      # whether a trailing comma is present-but-unwanted (`avoid_comma`) or
      # wanted-but-missing (`put_comma`), honouring `EnforcedStyleForMultiline`
      # (`no_comma` / `comma` / `consistent_comma` / `diff_comma`). For each
      # offense it returns the caret range, the message selector, and the single
      # corrector op (remove the trailing comma, or insert one after a range —
      # stock's `PunctuationCorrector.swap_comma`).
      #
      # The corrector op needs no Ruby string semantics, so it is fully computed
      # in Rust; the wrapper only turns it into the `corrector` call and selects
      # the message text. The `EnforcedStyleForMultiline` accessor is the genuine
      # `ConfigurableEnforcedStyle#style`, so an unrecognized style raises exactly
      # as stock does and `config_to_allow_offenses` is unaffected (this cop emits
      # no detection markers).
      #
      # Always bundle-eligible: the result is purely config-driven.
      class TrailingCommaInArguments < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        MSG = "%<command>s comma after the last %<unit>s."

        # Message selectors (mirror `trailing_comma_in_arguments.rs`).
        MSG_AVOID_NO_COMMA = 0
        MSG_AVOID_COMMA = 1
        MSG_AVOID_CONSISTENT_COMMA = 2
        MSG_AVOID_DIFF_COMMA = 3
        MSG_PUT = 4

        # Fully formatted messages, matching stock's `avoid_comma` /
        # `put_comma` (kind = 'parameter of %<article>s method call', article
        # 'a' for avoid and 'a multiline' for put, plus the style-specific
        # `extra_avoid_comma_info` suffix).
        MESSAGES = [
          format(MSG, command: "Avoid", unit: "parameter of a method call"),
          format(MSG, command: "Avoid",
                      unit: "parameter of a method call, unless each item is on its own line"),
          format(MSG, command: "Avoid",
                      unit: "parameter of a method call, unless items are split onto multiple lines"),
          format(MSG, command: "Avoid",
                      unit: "parameter of a method call, unless that item immediately precedes a newline"),
          format(MSG, command: "Put a", unit: "parameter of a multiline method call")
        ].freeze

        STYLES = {
          "no_comma" => 0,
          "comma" => 1,
          "consistent_comma" => 2,
          "diff_comma" => 3
        }.freeze

        # Corrector op kinds (mirror `trailing_comma_in_arguments.rs`).
        FIX_AVOID = 0
        FIX_PUT = 1

        def self.cop_name = "Style/TrailingCommaInArguments"
        def self.badge = RuboCop::Cop::Badge.parse("Style/TrailingCommaInArguments")

        # Packed config nums: `[style]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          # An unrecognized style defaults to no_comma here; the genuine error is
          # raised by the `style` accessor in `on_new_investigation`.
          [[STYLES.fetch(cop_config["EnforcedStyleForMultiline"] || "no_comma", 0)]]
        end

        def style_parameter_name
          "EnforcedStyleForMultiline"
        end

        def on_new_investigation
          # Validate `EnforcedStyleForMultiline` through the genuine accessor:
          # stock raises for an unrecognized style, and this must fire before we
          # derive the bundle config (which would otherwise mask it).
          style

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          resolved_result.each do |start, fin, message_idx, fix|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES[message_idx]) do |corrector|
              apply_fix(corrector, range, fix)
            end
          end
        end

        private

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :trailing_comma_in_arguments)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_trailing_comma_in_arguments(processed_source.raw_source, nums)
          end
        end

        # Always eligible (purely config-driven; no per-investigation state).
        def bundle_eligible?
          true
        end

        # Applies the corrector op. `swap_comma`: remove a `,` range, else insert
        # `,` after the range. Rust guarantees a `FIX_AVOID` range is exactly the
        # comma and a `FIX_PUT` range is not, so we can apply the op directly.
        def apply_fix(corrector, range, fix)
          case fix
          when FIX_AVOID
            corrector.remove(range)
          when FIX_PUT
            corrector.insert_after(range, ",")
          end
        end
      end
    end
  end
end
