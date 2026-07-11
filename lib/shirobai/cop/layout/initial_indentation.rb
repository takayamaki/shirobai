# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/InitialIndentation`.
      #
      # Rust answers only the cheap question — "is the first non-comment token
      # indented?" — with a leading-byte scan that never materializes the token
      # stream. On the overwhelming majority of files (column-0 start) the scan
      # settles it and this cop touches no tokens, which is the whole point:
      # stock's `first_token` forces the parser-gem token stream on EVERY file
      # (the "toucher" cost).
      #
      # Only when the scan reports an offense does the wrapper fall through to
      # stock's exact `first_token` / `space_before` construction (below,
      # verbatim from `vendor/rubocop/lib/rubocop/cop/layout/initial_indentation.rb`).
      # Because that construction is stock's own code, the offense range and the
      # autocorrect bytes are byte-identical by construction; the Rust scan is a
      # pure speed gate. The scan can only over-report (it never skips a token
      # stock keeps), and `space_before`'s `column.zero?` / no-space-to-the-left
      # guards filter any over-report back to stock's answer.
      class InitialIndentation < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Indentation of first line in file detected."

        def self.cop_name = "Layout/InitialIndentation"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to nums / lists. Kept for the
        # 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          return unless offense?

          space_before(first_token) do |space|
            add_offense(first_token.pos) do |corrector|
              corrector.remove(space)
            end
          end
        end

        private

        # Stock `first_token`: the first token whose text is not a `#` line
        # comment (block comments start with `=`, so they are kept).
        def first_token
          processed_source.tokens.find { |t| !t.text.start_with?("#") }
        end

        # Stock `space_before`, verbatim.
        def space_before(token)
          return unless token
          return if token.column.zero?

          space_range = range_with_surrounding_space(token.pos, side: :left, newlines: false)
          # A leading byte-order mark makes the column non-zero with no space to
          # the left; nothing to remove then.
          return if space_range == token.pos

          yield range_between(space_range.begin_pos, token.begin_pos)
        end

        def offense?
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :initial_indentation)
          else
            Shirobai.check_initial_indentation(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
