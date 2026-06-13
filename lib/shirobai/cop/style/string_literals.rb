# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/StringLiterals`.
      #
      # Rust walks every string literal once, replicating stock's two callbacks:
      # `StringHelp#on_str` (the per-`:str` quote check, with the begin-loc /
      # heredoc / ignored-node / interpolation guards and the
      # `correct_style_detected` markers) and `StringLiterals#on_dstr` (the
      # `ConsistentQuotesInMultiline` multi-line continued-string handling, whose
      # autocorrect is a stock no-op). For each record it returns, in walk order,
      # whether it offends, the caret range, the message selector, the detection
      # marker to replay, and — for an `on_str` offense — which autocorrect to
      # apply (`single` / `double`) plus the decoded string content.
      #
      # The replacement text is computed here with stock's genuine
      # `RuboCop::Cop::Util` helpers (`to_string_literal` / `String#inspect`),
      # because the quote conversion and escape handling are Ruby string
      # semantics; the detection markers are replayed through the genuine
      # `ConfigurableEnforcedStyle` methods so `config_to_allow_offenses` matches
      # stock exactly.
      #
      # Always bundle-eligible: the result is purely config-driven.
      class StringLiterals < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::Util
        extend RuboCop::Cop::AutoCorrector

        MSG_SINGLE = "Prefer single-quoted strings when you don't need string " \
                     "interpolation or special symbols."
        MSG_DOUBLE = "Prefer double-quoted strings unless you need single quotes to " \
                     "avoid extra backslashes for escaping."
        MSG_INCONSISTENT = "Inconsistent quote style."
        MESSAGES = [MSG_SINGLE, MSG_DOUBLE, MSG_INCONSISTENT].freeze

        STYLES = {
          "single_quotes" => 0,
          "double_quotes" => 1
        }.freeze

        # Fix kinds (mirror `string_literals.rs`).
        FIX_SINGLE = 0
        FIX_DOUBLE = 1
        FIX_NONE = 2

        def self.cop_name = "Style/StringLiterals"
        def self.badge = RuboCop::Cop::Badge.parse("Style/StringLiterals")

        # Packed config nums: `[style, consistent_multiline]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          # An unrecognized `EnforcedStyle` defaults to single here; the genuine
          # error is raised by the `style` accessor in `on_new_investigation`.
          [[
            STYLES.fetch(cop_config["EnforcedStyle"] || "single_quotes", 0),
            cop_config["ConsistentQuotesInMultiline"] ? 1 : 0
          ]]
        end

        def on_new_investigation
          # Validate `EnforcedStyle` through the genuine accessor first: stock
          # raises `RuntimeError` for an unrecognized style, and this must fire
          # before we derive the bundle config (which would otherwise raise a
          # `KeyError` for the unknown key).
          style

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          resolved_result.each do |is_offense, start, fin, message_idx, detect, fix, content|
            unless is_offense
              replay_detection(detect)
              next
            end

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES[message_idx]) do |corrector|
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

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :string_literals)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_string_literals(processed_source.raw_source, nums)
          end
        end

        # Always eligible (purely config-driven; no per-investigation state).
        def bundle_eligible?
          true
        end

        # Replays stock's detection side effect: `opposite_style_detected` for an
        # offense, `correct_style_detected` for a non-offending `on_str` node.
        # `dstr` offenses and the inconsistent case carry no marker.
        def replay_detection(detect)
          case detect
          when 0 then opposite_style_detected
          when 1 then correct_style_detected
          end
        end

        # Applies the autocorrect for an `on_str` offense. `dstr` offenses
        # (`FIX_NONE`) yield the block but leave the corrector untouched, exactly
        # like stock's `StringLiteralCorrector.correct` returning for a `dstr`.
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
