# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/OrderedMagicComments`.
      #
      # Rust reproduces stock's `magic_comment_lines` scan over the same leading
      # comment lines as `Lint/DuplicateMagicComment` (`leading_comment_lines`
      # from the `FrozenStringLiteral` mixin) and the `MagicComment.parse`
      # `encoding_specified?` / `elsif valid?` bucketing. It returns at most one
      # offense as the two 1-based line numbers `(encoding_line, other_line)`
      # when the encoding magic comment does NOT precede the other magic comment.
      #
      # The wrapper rebuilds the offense range and the line swap with stock's own
      # `buffer.line_range`, so detection and autocorrect are byte-identical by
      # construction. Line numbers are byte/char agnostic, so no `SourceOffsets`
      # conversion is needed; `line_range` is only built on offense files (stock
      # asks the first token for its `line` — materializing `Buffer#line_begins`
      # — on every file).
      class OrderedMagicComments < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "The encoding magic comment should precede all other magic comments."

        def self.cop_name = "Lint/OrderedMagicComments"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to nums / lists. Kept for the
        # 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          return if buffer.source.empty?

          resolved_offense.each do |encoding_line, other_line|
            range = buffer.line_range(encoding_line)
            add_offense(range) do |corrector|
              range2 = buffer.line_range(other_line)
              corrector.replace(range, range2.source)
              corrector.replace(range2, range.source)
            end
          end
        end

        private

        def resolved_offense
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :ordered_magic_comments)
          else
            Shirobai.check_ordered_magic_comments(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
