# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/TrailingCommaInHashLiteral`.
      #
      # Rust walks every braced hash literal once, replicating stock's
      # `on_hash` and the shared `TrailingComma#check_literal` mixin: it
      # decides whether a trailing comma is present-but-unwanted
      # (`avoid_comma`) or wanted-but-missing (`put_comma`), honouring
      # `EnforcedStyleForMultiline` (`no_comma` / `comma` / `consistent_comma`
      # / `diff_comma`). A braceless keyword hash is a different Prism node
      # type and never reaches this cop — stock skips it the same way (no
      # `loc.end`) because `Style/TrailingCommaInArguments` checks it.
      #
      # For each offense Rust returns the caret range, the message selector,
      # and the single corrector op (remove the trailing comma, or insert one
      # after a range — stock's `PunctuationCorrector.swap_comma`). The wrapper
      # only turns the op into the corrector call and selects the message text.
      # The `EnforcedStyleForMultiline` accessor is the genuine
      # `ConfigurableEnforcedStyle#style`, so an unrecognized style raises
      # exactly as stock does.
      #
      # Always bundle-eligible: the result is purely config-driven.
      class TrailingCommaInHashLiteral < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        MSG = "%<command>s comma after the last %<unit>s."

        # Message selectors (mirror `trailing_comma.rs`).
        MSG_AVOID_NO_COMMA = 0
        MSG_AVOID_COMMA = 1
        MSG_AVOID_CONSISTENT_COMMA = 2
        MSG_AVOID_DIFF_COMMA = 3
        MSG_PUT = 4

        # Fully formatted messages, matching stock's `avoid_comma` /
        # `put_comma` (kind = 'item of %<article>s hash', article 'a' for
        # avoid and 'a multiline' for put, plus the style-specific
        # `extra_avoid_comma_info` suffix).
        MESSAGES = [
          format(MSG, command: "Avoid", unit: "item of a hash"),
          format(MSG, command: "Avoid",
                      unit: "item of a hash, unless each item is on its own line"),
          format(MSG, command: "Avoid",
                      unit: "item of a hash, unless items are split onto multiple lines"),
          format(MSG, command: "Avoid",
                      unit: "item of a hash, unless that item immediately precedes a newline"),
          format(MSG, command: "Put a", unit: "item of a multiline hash")
        ].freeze

        STYLES = {
          "no_comma" => 0,
          "comma" => 1,
          "consistent_comma" => 2,
          "diff_comma" => 3
        }.freeze

        # Corrector op kinds (mirror `trailing_comma.rs`).
        FIX_AVOID = 0
        FIX_PUT = 1

        def self.cop_name = "Style/TrailingCommaInHashLiteral"
        def self.badge = RuboCop::Cop::Badge.parse("Style/TrailingCommaInHashLiteral")

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
            Dispatch.offenses_for(processed_source, config, :trailing_comma_in_hash_literal)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_trailing_comma_in_hash_literal(processed_source.raw_source, nums)
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
