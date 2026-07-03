# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideHashLiteralBraces`.
      #
      # Rust walks the AST once, replicating stock's `on_hash` /
      # `on_hash_pattern` over hash literals and braced hash patterns: the
      # left-brace check (skipped across a line break or before a comment),
      # the right-brace check, and the whitespace-only-inner check under the
      # `no_space` empty style. The `compact` style's same-brace-token test on
      # the right (`tRCURLY` vs a `%w{...}` `tSTRING_END`) is resolved from
      # the collected hash / hash-pattern / brace-block / brace-lambda closing
      # positions.
      #
      # Each offense comes back as `[start, end, message_code]`. The corrector
      # follows stock's `autocorrect`, keyed on the live `range.source`: a
      # whitespace range is removed, a `{` gets a space after, anything else a
      # space before. Stock can emit the same range twice for `{ }` (the
      # left-brace check and the whitespace-only check); `add_offense` dedups
      # by location on both sides, so the duplicate is forwarded as-is.
      class SpaceInsideHashLiteralBraces < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Space inside { missing.",
          "Space inside { detected.",
          "Space inside } missing.",
          "Space inside } detected.",
          "Space inside empty hash literal braces missing.",
          "Space inside empty hash literal braces detected."
        ].freeze

        STYLES = { "space" => 0, "no_space" => 1, "compact" => 2 }.freeze

        def self.cop_name = "Layout/SpaceInsideHashLiteralBraces"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [EnforcedStyle, EnforcedStyleForEmptyBraces == 'no_space']`
        # and no lists.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            [
              STYLES.fetch(cop_config["EnforcedStyle"] || "space"),
              cop_config["EnforcedStyleForEmptyBraces"] == "no_space" ? 1 : 0
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
              case range.source
              when /\s/ then corrector.remove(range)
              when "{" then corrector.insert_after(range, " ")
              else corrector.insert_before(range, " ")
              end
            end
          end
        end

        private

        def offenses_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_inside_hash_literal_braces)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_inside_hash_literal_braces(
              processed_source.raw_source, nums[0], nums[1] == 1
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
