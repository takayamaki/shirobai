# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAroundKeyword`.
      #
      # Rust walks the AST and reproduces every stock `on_xxx` callback (if /
      # unless / while / until / case / for / block / begin / super / yield /
      # return / break / next / defined? / and / or / not / rescue / ensure /
      # when / in / BEGIN / END), checking the space before and/or after each
      # keyword range with the same character rules as stock (the
      # `space_before_missing?` / `space_after_missing?` accept sets, the
      # ACCEPT_LEFT_PAREN / ACCEPT_LEFT_SQUARE_BRACKET / namespace / safe-
      # navigation exceptions, and the `preceded_by_operator?` ancestor walk).
      #
      # It returns, per offense, the keyword range plus a `before` flag: `true`
      # is a missing space *before* the keyword (autocorrect inserts a space
      # before the range, `MSG_BEFORE`), `false` a missing space *after* it
      # (inserts a space after, `MSG_AFTER`).
      #
      # The cop has no configuration, so it is always bundle eligible; the
      # offenses come from the per-file bundled run (`Shirobai::Dispatch`). The
      # autocorrect re-passes re-investigate a fresh `ProcessedSource`, which
      # recomputes the bundle from scratch, so this cop keeps no cross-pass
      # state.
      class SpaceAroundKeyword < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG_BEFORE = "Space before keyword `%<range>s` is missing."
        MSG_AFTER = "Space after keyword `%<range>s` is missing."

        def self.cop_name = "Layout/SpaceAroundKeyword"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No packed configuration: the cop contributes nothing to the bundle's
        # `(nums, lists)` wire format. Kept for symmetry with the other wrappers.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          offenses_for_source.each do |start, fin, before|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            if before
              add_offense(range, message: format(MSG_BEFORE, range: range.source)) do |corrector|
                corrector.insert_before(range, " ")
              end
            else
              add_offense(range, message: format(MSG_AFTER, range: range.source)) do |corrector|
                corrector.insert_after(range, " ")
              end
            end
          end
        end

        private

        def offenses_for_source
          Dispatch.offenses_for(processed_source, config, :space_around_keyword)
        end
      end
    end
  end
end
