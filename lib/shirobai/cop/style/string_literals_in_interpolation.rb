# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/StringLiteralsInInterpolation`.
      #
      # The interpolation counterpart of `Style/StringLiterals`: Rust walks every
      # string literal once, replicating stock's `StringHelp#on_str` with this
      # cop's *inverted* interpolation guard. A `:str` node offends iff it is
      # inside an interpolation (`#{...}` of a string / symbol / regexp) and its
      # quotes are `wrong_quotes?` for the configured `EnforcedStyle`; a
      # non-offending `:str` with a begin loc emits a `correct_style_detected`
      # marker. Heredoc and physical-newline (multi-line) `:str` nodes are skipped
      # exactly as parser-gem skips them (no begin loc / `:dstr` split).
      #
      # For each record Rust returns, in walk order, whether it offends, the caret
      # range, the detection marker to replay, and — for an offense — which
      # autocorrect to apply (`single` / `double`) plus the decoded string
      # content. The replacement text is computed here with stock's genuine
      # `RuboCop::Cop::Util` helpers (`to_string_literal` / `String#inspect`),
      # and the detection markers are replayed through the genuine
      # `ConfigurableEnforcedStyle` methods so `config_to_allow_offenses` matches
      # stock exactly.
      #
      # Always bundle-eligible: the result is purely config-driven.
      class StringLiteralsInInterpolation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Util
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "single_quotes" => 0,
          "double_quotes" => 1
        }.freeze

        # Fix kinds (mirror `string_literals_in_interpolation.rs`).
        FIX_SINGLE = 0
        FIX_DOUBLE = 1

        def self.cop_name = "Style/StringLiteralsInInterpolation"
        def self.badge = RuboCop::Cop::Badge.parse("Style/StringLiteralsInInterpolation")

        # Packed config nums: `[style]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          # An unrecognized `EnforcedStyle` defaults to single here; the genuine
          # error is raised by the `style` accessor in `on_new_investigation`.
          [[STYLES.fetch(cop_config["EnforcedStyle"] || "single_quotes", 0)]]
        end

        def on_new_investigation
          # Validate `EnforcedStyle` through the genuine accessor first: stock
          # raises `RuntimeError` for an unrecognized style, and this must fire
          # before we derive the bundle config (which would otherwise raise a
          # `KeyError` for the unknown key).
          style

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          resolved_result.each do |is_offense, start, fin, detect, fix, content|
            unless is_offense
              replay_detection(detect)
              next
            end

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              apply_fix(corrector, range, fix, content)
              replay_detection(detect)
            end
          end
        end

        def alternative_style
          case style
          when :single_quotes
            :double_quotes
          else
            :single_quotes
          end
        end

        private

        def message
          # single_quotes -> single-quoted
          kind = style.to_s.sub(/_(.*)s/, '-\1d')

          "Prefer #{kind} strings inside interpolations."
        end

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :string_literals_in_interpolation)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_string_literals_in_interpolation(processed_source.raw_source, nums)
          end
        end

        # Always eligible (purely config-driven; no per-investigation state).
        def bundle_eligible?
          true
        end

        # Replays stock's detection side effect: `opposite_style_detected` for an
        # offense, `correct_style_detected` for a non-offending `on_str` node.
        def replay_detection(detect)
          case detect
          when 0 then opposite_style_detected
          when 1 then correct_style_detected
          end
        end

        # Applies the autocorrect, mirroring `StringLiteralCorrector.correct`:
        # `single` -> `to_string_literal`, `double` -> `String#inspect`.
        def apply_fix(corrector, range, fix, content)
          case fix
          when FIX_SINGLE
            corrector.replace(range, to_string_literal(content))
          when FIX_DOUBLE
            corrector.replace(range, content.inspect)
          end
        end
      end
    end
  end
end
