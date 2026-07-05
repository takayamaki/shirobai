# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyComment`.
      #
      # Rust pulls every comment range out of the shared parse cache, strips
      # whitespace from each comment's text, appends `"\n"`, and pattern-matches
      # either the joined text of a chunk (`AllowMarginComment: true`, the
      # default) or each comment alone (`AllowMarginComment: false`). The
      # `AllowBorderComment` flag switches the pattern between `/\A(#\n)+\z/`
      # and `/\A(#+\n)+\z/` — byte-for-byte stock semantics. Chunking groups
      # consecutive comments whose lines differ by 1 AND start at the same
      # column (block comments naturally break the chain because they span
      # multiple lines).
      #
      # Autocorrect: when an earlier token shares the comment's line, drop the
      # comment plus its leading horizontal whitespace
      # (`range_with_surrounding_space(newlines: false)`); otherwise drop the
      # whole line(s) the comment occupies, including the final newline
      # (`range_by_whole_lines(include_final_newline: true)`). Both shapes are
      # produced on the Rust side as byte ranges, so the Ruby wrapper is just a
      # thin tuple-to-`Parser::Source::Range` conversion.
      class EmptyComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Source code comment is empty."

        def self.cop_name = "Layout/EmptyComment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[allow_border, allow_margin]`.
        def self.bundle_args(config)
          cfg = config.for_badge(badge)
          allow_border = cfg["AllowBorderComment"]
          allow_margin = cfg["AllowMarginComment"]
          allow_border = true if allow_border.nil?
          allow_margin = true if allow_margin.nil?
          [allow_border ? 1 : 0, allow_margin ? 1 : 0]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          # On the bundle path Rust scanned `raw_source` (byte-identical to
          # `buffer.source`); on the CRLF/BOM fallback the standalone path
          # scans `buffer.source` instead so every offset Rust returns lines
          # up with the parser-gem buffer.
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin, ac_start, ac_end|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            ac_range = Parser::Source::Range.new(buffer, off[ac_start], off[ac_end])
            add_offense(range, message: MSG) do |corrector|
              corrector.remove(ac_range)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :empty_comment)
          else
            args = self.class.bundle_args(config)
            Shirobai.check_empty_comment(processed_source.buffer.source, args[0] != 0, args[1] != 0)
          end
        end
      end
    end
  end
end
