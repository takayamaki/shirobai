# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ExtraSpacing`.
      #
      # A token-scan cop with an AST side input: Rust walks the parser-gem token
      # stream as adjacent pairs (`tokens.each_cons(2)`, stock's `check_tokens`)
      # and, for the `AllowForAlignment` / `ignored_ranges` decisions, consults
      # the whole token list (the shared alignment helper) and the AST (the
      # key↔value spans of multi-line hash pairs, left to `Layout/HashAlignment`).
      # Both run in the bundle's walk-outer phase; Rust returns one record per
      # offense: `[start, end, message, action, edits]`.
      #
      # `[start, end)` is the offense highlight. `action` is `0`
      # (`corrector.remove(range)` — the usual "delete extra spacing" fix) or `1`
      # (`ForceEqualSignAlignment`: apply each `[edit_start, edit_end, text]` of
      # `edits` with `corrector.replace` — a zero-width range inserts, an empty
      # `text` deletes; Rust has already applied the `@corrected` dedup so each
      # token's edit appears under exactly one offense).
      #
      # Config inputs: `AllowForAlignment` (default true) /
      # `AllowBeforeTrailingComments` (default false) / `ForceEqualSignAlignment`
      # (default false). All are purely config-driven, so the cop is always bundle
      # eligible — except when `raw_source != buffer.source` (CRLF/BOM
      # normalization), where the token byte positions would not line up and it
      # falls back to a standalone lex of `buffer.source`. (CRLF/buffer.source
      # trap, `aee0e8e`.)
      class ExtraSpacing < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/ExtraSpacing"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed config nums: `[enabled, allow_for_alignment,
        # allow_before_trailing_comments, force_equal_sign_alignment]`.
        #
        # `enabled` is a token-cop gate the bundle ORs into its decision to
        # collect the parser-gem token stream. It is a superset of "this cop
        # runs": a false positive only costs the token pass on a file the cop
        # never reads, never a wrong offense, so we only turn it off when
        # `Enabled` is literally `false`.
        #
        # `force_equal_sign_alignment` is the same config key
        # (`Layout/ExtraSpacing` `ForceEqualSignAlignment`) that
        # `Layout/SpaceAroundOperators` reads; the bundle packs it once (num 126)
        # as the single wire source. It is carried here for the standalone
        # fallback call below.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          enabled = cop_config["Enabled"] == false ? 0 : 1
          allow = cop_config.fetch("AllowForAlignment", true) ? 1 : 0
          trailing = cop_config["AllowBeforeTrailingComments"] ? 1 : 0
          force = cop_config["ForceEqualSignAlignment"] ? 1 : 0
          [[enabled, allow, trailing, force]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source_for_offsets = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source_for_offsets)
          ranges = ignored_ranges(source_for_offsets)

          resolved_result.each do |start_pos, end_pos, message, action, edits|
            # `ignored_range?(ast, range.begin_pos)`: applied here (not in Rust)
            # with the memoized `@ignored_ranges`, reproducing stock's instance
            # memoization — across an autocorrect re-pass on a reused instance the
            # ranges go stale exactly as stock's do. Only the "remove extra space"
            # action (0) is subject to it; the ForceEqualSignAlignment action (1)
            # is not.
            next if action == 0 && ranges.any? { |s, e| (s...e).cover?(start_pos) }

            range = Parser::Source::Range.new(buffer, off[start_pos], off[end_pos])
            add_offense(range, message: message) do |corrector|
              if action == 0
                corrector.remove(range)
              else
                edits.each do |edit_start, edit_end, text|
                  edit_range = Parser::Source::Range.new(buffer, off[edit_start], off[edit_end])
                  corrector.replace(edit_range, text)
                end
              end
            end
          end
        end

        private

        # `@ignored_ranges ||= …` — stock memoizes the multi-line hash key↔value
        # spans on the cop instance, so a reused instance (the autocorrect
        # re-pass loop in `expect_correction`, and any tool driving cops directly)
        # keeps the first source's byte offsets. The real CLI builds a fresh team
        # per pass, so it never sees the stale ranges; reproducing the
        # memoization keeps shirobai identical under both.
        def ignored_ranges(source)
          @ignored_ranges ||= Shirobai.extra_spacing_ignored_ranges(source)
        end

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :extra_spacing)
          else
            # nums[0] is the token-cop gate (only the bundle reads it); the
            # standalone fallback runs unconditionally, so skip it and pass the
            # three config nums (allow / trailing / force).
            nums = self.class.bundle_args(config).first
            Shirobai.check_extra_spacing(
              processed_source.buffer.source,
              nums[1] != 0, nums[2] != 0, nums[3] != 0
            )
          end
        end

        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
