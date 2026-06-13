# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/HashSyntax`.
      #
      # Rust walks every hash literal once, replicating stock's `on_hash`
      # (the `EnforcedStyle` `check` machinery for `ruby19` / `hash_rockets` /
      # `no_mixed_keys` / `ruby19_no_mixed_keys`) and `HashShorthandSyntax`'s
      # `on_hash_for_mixed_shorthand` (`consistent` / `either_consistent`) plus
      # `on_pair` (`always` / `never`). It returns, in walk order, one record per
      # offending pair (with the message selector and the exact corrector op
      # sequence) and one per *non-offending* pair under an `EnforcedStyle` check
      # (a `correct_style_detected` marker). The wrapper applies the ops verbatim
      # and replays the detection side effects through the genuine
      # `ConfigurableEnforcedStyle` methods so `config_to_allow_offenses`
      # matches stock exactly.
      #
      # Symbol acceptability (`acceptable_19_syntax_symbol?`) is decided in Rust
      # by an ASCII byte port of stock's regexes (the gem's `\w` is ASCII-only
      # here, so non-ASCII symbol names never convert) — no Ruby-side regex.
      #
      # Always bundle-eligible: the result is purely config-driven.
      class HashSyntax < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Use the new Ruby 1.9 hash syntax.",
          "Use hash rockets syntax.",
          "Don't mix styles in the same hash.",
          "Omit the hash value.",
          "Include the hash value.",
          "Do not mix explicit and implicit hash values. Omit the hash value.",
          "Do not mix explicit and implicit hash values. Include the hash value."
        ].freeze

        STYLES = {
          "ruby19" => 0,
          "hash_rockets" => 1,
          "no_mixed_keys" => 2,
          "ruby19_no_mixed_keys" => 3
        }.freeze
        SHORTHAND = {
          "always" => 0,
          "never" => 1,
          "either" => 2,
          "consistent" => 3,
          "either_consistent" => 4
        }.freeze

        def self.cop_name = "Style/HashSyntax"
        def self.badge = RuboCop::Cop::Badge.parse("Style/HashSyntax")

        # Packed config nums: `[style, shorthand,
        # UseHashRocketsWithSymbolValues, PreferHashRocketsForNonAlnumEndingSymbols,
        # ruby31_plus, ruby22_plus]`. The two ruby-version flags come from
        # `AllCops/TargetRubyVersion` (the cop's `target_ruby_version`), gating
        # the shorthand checks (`> 3.0`) and quoted-symbol ruby19 keys (`> 2.1`).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          target = config.target_ruby_version
          [[
            STYLES.fetch(cop_config["EnforcedStyle"] || "ruby19"),
            SHORTHAND.fetch(cop_config.fetch("EnforcedShorthandSyntax", "always")),
            cop_config["UseHashRocketsWithSymbolValues"] ? 1 : 0,
            cop_config["PreferHashRocketsForNonAlnumEndingSymbols"] ? 1 : 0,
            target > 3.0 ? 1 : 0,
            target > 2.1 ? 1 : 0
          ]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          records = resolved_result

          records.each do |is_offense, start, fin, message_idx, detect, ops|
            unless is_offense
              # `correct_style_detected` marker (a non-offending pair).
              correct_style_detected
              next
            end

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES[message_idx]) do |corrector|
              apply_ops(corrector, buffer, off, ops)
              replay_detection(detect)
            end
          end
        end

        # `Style/SpaceAroundOperators`-style autocorrect interferes within the
        # same range; the autocorrect loop resolves clobbering. Declared like
        # stock (no special incompatibility).

        def alternative_style
          case style
          when :hash_rockets
            :ruby19
          when :ruby19, :ruby19_no_mixed_keys
            :hash_rockets
          end
        end

        private

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :hash_syntax)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_hash_syntax(processed_source.raw_source, nums)
          end
        end

        # Always eligible (purely config-driven; no per-investigation state).
        def bundle_eligible?
          true
        end

        # Replays stock's detection side effect for an offense.
        def replay_detection(detect)
          case detect
          when 0 then opposite_style_detected
          when 2 then self.config_to_allow_offenses = { "Enabled" => false }
            # 1 (correct_style) is only emitted for non-offense markers; 3 (none)
            # has no side effect.
          end
        end

        def apply_ops(corrector, buffer, off, ops)
          ops.each do |kind, s, e, text|
            range = Parser::Source::Range.new(buffer, off[s], off[e])
            case kind
            when 0 then corrector.replace(range, text)
            when 1 then corrector.remove(range)
            when 2 then corrector.insert_before(range, text)
            when 3 then corrector.insert_after(range, text)
            end
          end
        end
      end
    end
  end
end
