# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/DuplicateMagicComment`.
      #
      # Rust reproduces the whole detection: the `leading_comment_lines`
      # prefix (lines strictly before the first non-comment token, or every
      # line when the file has none) and the `MagicComment.parse`
      # classification of each line into the encoding /
      # frozen_string_literal buckets. It returns the 1-based line numbers
      # of every duplicate (each bucket's lines after the first). The
      # wrapper rebuilds the offense range and the whole-line removal with
      # stock's own helpers (`buffer.line_range` + `range_by_whole_lines`),
      # so detection and autocorrect are byte-identical by construction.
      #
      # Unlike `Layout/EmptyLineAfterMagicComment`, the magic-comment
      # regexes are ported to Rust rather than reused from Ruby: this cop's
      # payoff is skipping stock's `first_non_comment_token.line` call,
      # whose `Buffer#line_begins` materialization is quadratic on large
      # non-ASCII files (String#index re-scans from the start per line;
      # ~1.5s on Discourse's `test_data.rb`). Running `MagicComment.parse`
      # over a whale file's leading lines on the Ruby side would keep a
      # large part of that cost. Line numbers are byte/char agnostic, so no
      # `SourceOffsets` conversion is needed; `line_range` is only built on
      # offense files (stock pays `line_begins` on every file).
      class DuplicateMagicComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Duplicate magic comment detected."

        def self.cop_name = "Lint/DuplicateMagicComment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to nums / lists. Kept for the
        # 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          return if processed_source.buffer.source.empty?

          resolved_lines.each do |line|
            range = processed_source.buffer.line_range(line)
            add_offense(range) do |corrector|
              corrector.remove(range_by_whole_lines(range, include_final_newline: true))
            end
          end
        end

        private

        def resolved_lines
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :duplicate_magic_comment)
          else
            Shirobai.check_duplicate_magic_comment(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
