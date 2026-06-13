# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideBlockBraces`.
      #
      # Rust walks the AST once, replicating stock's `on_block` (plus the
      # `numblock` / `itblock` aliases): it skips `do`..`end` blocks and
      # multi-line empty braces, then judges the inside-brace spacing per
      # `EnforcedStyle`, the empty-brace spacing per `EnforcedStyleForEmptyBraces`
      # and the `{`-to-`|` spacing per `SpaceBeforeBlockParameters`, using the
      # same character rules and range arithmetic as stock
      # (`range_with_surrounding_space`, the multi-line / `]`-ending right-brace
      # cases, `aligned_braces?`).
      #
      # Each offense comes back as `[start, end, message_code]`. The message and
      # the corrector action both follow stock's `offense` method, which derives
      # the action from the live `range.source`: a whitespace range is removed,
      # `{}` becomes `{ }`, `{|` becomes `{ |`, otherwise a space is inserted
      # before the range. Because the action is recomputed from the fresh source
      # on every autocorrect pass (the bundle is recomputed per pass), the cop
      # keeps no cross-pass state.
      class SpaceInsideBlockBraces < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        # Stock declares the same incompatibility: both cops can rewrite the same
        # brace, so RuboCop's correction loop defers a conflicting pass.
        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::BlockDelimiters]
        end

        MESSAGES = [
          "Space missing inside {.",
          "Space inside { detected.",
          "Space missing inside }.",
          "Space inside } detected.",
          "Space inside empty braces detected.",
          "Space missing inside empty braces.",
          "Space between { and | missing.",
          "Space between { and | detected."
        ].freeze

        STYLES = { "space" => 0, "no_space" => 1 }.freeze

        def self.cop_name = "Layout/SpaceInsideBlockBraces"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [EnforcedStyle, EnforcedStyleForEmptyBraces,
        # SpaceBeforeBlockParameters]` and no lists.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            [
              STYLES.fetch(cop_config["EnforcedStyle"] || "space"),
              STYLES.fetch(cop_config["EnforcedStyleForEmptyBraces"] || "no_space"),
              cop_config["SpaceBeforeBlockParameters"] == false ? 0 : 1
            ],
            []
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          offenses_for_source.each do |start, fin, code|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES.fetch(code)) do |corrector|
              # Stock's `offense` corrector, keyed on the live range source.
              case range.source
              when /\s/ then corrector.remove(range)
              when "{}" then corrector.replace(range, "{ }")
              when "{|" then corrector.replace(range, "{ |")
              else           corrector.insert_before(range, " ")
              end
            end
          end
        end

        private

        def offenses_for_source
          validate_empty_braces_style!
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_inside_block_braces)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_inside_block_braces(
              processed_source.raw_source, nums[0], nums[1], nums[2] == 1
            )
          end
        end

        # Mirrors stock's `style_for_empty_braces`, which raises this exact
        # message for any value outside `space` / `no_space`. (Stock raises
        # lazily from the empty-brace branch; real runs always pass a valid
        # style, so validating up front matches every observable case.)
        def validate_empty_braces_style!
          value = config.for_badge(self.class.badge)["EnforcedStyleForEmptyBraces"]
          return if value.nil? || STYLES.key?(value.to_s)

          raise "Unknown EnforcedStyleForEmptyBraces selected!"
        end

        # No per-investigation state and no config that can't be packed, so this
        # cop is always bundle eligible. Kept for symmetry / a future fallback.
        def bundle_eligible?
          true
        end
      end
    end
  end
end
