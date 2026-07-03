# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeBlockBraces`.
      #
      # Rust walks the AST once and decides every offense (`do ... end` and
      # the `Style/BlockDelimiters` multiline conflict already excluded),
      # sending back only the offense ranges plus a five-flag style-detection
      # summary — an earlier version shipped one record per brace block and
      # decided here, which scaled the wire volume with the number of blocks.
      #
      # The summary replays stock's `config_to_allow_offenses` bookkeeping
      # byte-identically (the vendor spec asserts it):
      #
      # - non-empty blocks matching the style fire `correct_style_detected`
      #   once (the state machine intersects sets, so repeats are idempotent);
      # - each non-empty offense fires `opposite_style_detected` inside its
      #   `add_offense` block, exactly like stock (so directive-disabled lines
      #   suppress it);
      # - the empty-braces axis writes one fixed value and can only be cleared
      #   by a matching empty block seen while a value is stored, so "match
      #   before first offense" / "match after some offense" bits reproduce
      #   every ordering (`no_acceptable_style!` replaces the whole hash and
      #   freezes all later events, making the flag replay exact).
      #
      # The lazy `Unknown EnforcedStyleForEmptyBraces selected!` raise fires
      # when an empty-brace block was seen under an invalid setting, mirroring
      # stock's raise from `check_empty`.
      class SpaceBeforeBlockBraces < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MISSING_MSG = "Space missing to the left of {."
        DETECTED_MSG = "Space detected to the left of {."

        STYLES = { "space" => 0, "no_space" => 1 }.freeze

        # Stock declares the same incompatibility: both cops can rewrite the
        # same brace, so RuboCop's correction loop defers a conflicting pass.
        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::SymbolProc]
        end

        def self.cop_name = "Layout/SpaceBeforeBlockBraces"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [EnforcedStyle, resolved EnforcedStyleForEmptyBraces
        # (0 space / 1 no_space / 2 invalid; nil follows EnforcedStyle),
        # Style/BlockDelimiters EnforcedStyle == 'line_count_based']`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          style = STYLES.fetch(cop_config["EnforcedStyle"] || "space")
          empty = case cop_config["EnforcedStyleForEmptyBraces"]
                  when "space" then 0
                  when "no_space" then 1
                  when nil then style
                  else 2
                  end
          bd = config.for_cop("Style/BlockDelimiters")["EnforcedStyle"] == "line_count_based"
          [[style, empty, bd ? 1 : 0], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses, summary = result_for_source
          a_correct, b_match_first, b_offense, b_match_after, saw_empty = summary

          # Stock raises lazily from check_empty when an empty-brace block is
          # reached under an invalid EnforcedStyleForEmptyBraces.
          raise "Unknown EnforcedStyleForEmptyBraces selected!" if saw_empty && invalid_empty_style?

          replay_empty_braces_axis(b_match_first, b_offense, b_match_after)
          correct_style_detected if a_correct

          offenses.each do |start, fin, detected, from_empty|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: detected ? DETECTED_MSG : MISSING_MSG) do |corrector|
              autocorrect(corrector, range)
              opposite_style_detected unless from_empty
            end
          end
        end

        private

        # Replays check_empty's config_to_allow_offenses effects in an order
        # equivalent to the in-file event order (see the class docs).
        def replay_empty_braces_axis(b_match_first, b_offense, b_match_after)
          handle_different_styles_for_empty_braces(style_for_empty_braces) if b_match_first
          if b_offense && !config_to_allow_offenses.key?("Enabled")
            used_style = style_for_empty_braces == :space ? :no_space : :space
            config_to_allow_offenses["EnforcedStyleForEmptyBraces"] = used_style.to_s
          end
          handle_different_styles_for_empty_braces(style_for_empty_braces) if b_match_after
        end

        def handle_different_styles_for_empty_braces(used_style)
          if config_to_allow_offenses["EnforcedStyleForEmptyBraces"] &&
             config_to_allow_offenses["EnforcedStyleForEmptyBraces"].to_sym != used_style
            config_to_allow_offenses.clear
            config_to_allow_offenses["Enabled"] = false
          end
        end

        def autocorrect(corrector, range)
          case range.source
          when /\s/ then corrector.remove(range)
          else corrector.insert_before(range, " ")
          end
        end

        def style_for_empty_braces
          case cop_config["EnforcedStyleForEmptyBraces"]
          when "space" then :space
          when "no_space" then :no_space
          when nil then style
          else raise "Unknown EnforcedStyleForEmptyBraces selected!"
          end
        end

        def invalid_empty_style?
          !["space", "no_space", nil].include?(cop_config["EnforcedStyleForEmptyBraces"])
        end

        def result_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_before_block_braces)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_before_block_braces(
              processed_source.raw_source, nums[0], nums[1], nums[2] == 1
            )
          end
        end

        # No per-investigation state and no config that can't be packed, so
        # this cop is always bundle eligible. Kept for symmetry / a future
        # fallback.
        def bundle_eligible?
          true
        end
      end
    end
  end
end
