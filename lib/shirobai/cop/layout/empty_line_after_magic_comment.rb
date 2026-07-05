# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLineAfterMagicComment`.
      #
      # Rust pulls every comment that appears before the file's first AST
      # statement (or every comment when there are no statements) from the
      # shared parse cache and returns each as `(start, end, line)`. The Ruby
      # wrapper finishes step 2 of stock's `on_new_investigation`: filter the
      # candidates via `MagicComment.parse(text).any?`, take the LAST match,
      # and check the line right below it (`processed_source[line]`). When
      # that line exists and is not blank after `strip`, emit an offense at
      # `source_range(buffer, line + 1, 0)` with `corrector.insert_before(
      # range, "\n")` — byte-identical to stock.
      #
      # `MagicComment.parse` is regex-heavy and supports three formats
      # (SimpleComment / EmacsComment / VimComment); keeping it on the Ruby
      # side reuses stock's implementation verbatim and avoids drift.
      class EmptyLineAfterMagicComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Add an empty line after magic comments."

        def self.cop_name = "Layout/EmptyLineAfterMagicComment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          [] # config-less
        end

        def on_new_investigation
          candidates = resolved_candidates
          return if candidates.empty?

          # Find the last magic comment in document order (`reverse.find`).
          # The candidate slices come from the same buffer Rust scans
          # (raw_source on the bundle path, buffer.source on the fallback);
          # both are byte-identical to the bytes Rust used to compute the
          # ranges, so plain byte indexing is safe.
          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          last = nil
          candidates.reverse_each do |start, fin, line|
            text = source.byteslice(start, fin - start)
            next unless text && RuboCop::MagicComment.parse(text).any?

            last = [start, fin, line]
            break
          end
          return unless last

          _start, _fin, magic_line = last
          # `processed_source[magic_line]` (0-indexed access) is the line just
          # below the magic comment. `nil` when the magic is the file's final
          # line — stock returns in that case.
          next_line = processed_source[magic_line]
          return if next_line.nil?
          return if next_line.strip.empty?

          buffer = processed_source.buffer
          offending_range = source_range(buffer, magic_line + 1, 0)
          add_offense(offending_range) do |corrector|
            corrector.insert_before(offending_range, "\n")
          end
        end

        private

        def resolved_candidates
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :empty_line_after_magic_comment)
          else
            Shirobai.check_empty_line_after_magic_comment(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
