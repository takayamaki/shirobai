# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/LeadingEmptyLines`.
      #
      # Stock's `on_new_investigation`:
      #
      #   1. `token = processed_source.tokens[0]`.
      #   2. `return unless token && token.line > 1`.
      #   3. Emit one offense at `token.pos` whose corrector removes the
      #      range `[0, token.begin_pos)`.
      #
      # The first token covers code keywords/identifiers, inline `#` comments,
      # `=begin/=end` block comments, and (essentially) anything parser-gem's
      # tokenizer reports. Files starting with `__END__` carry no tokens at
      # all, so the cop stays silent even when leading blank lines precede the
      # marker.
      #
      # Rust returns at most one tuple `[start, end, ac_start, ac_end]` (all
      # byte offsets into the raw source); the wrapper just turns them into
      # `Parser::Source::Range`s and attaches the corrector. The byte→char
      # conversion runs through `SourceOffsets` so BOM / multibyte files line
      # up with parser-gem's character buffer indexing.
      class LeadingEmptyLines < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Unnecessary blank line at the beginning of the source."

        def self.cop_name = "Layout/LeadingEmptyLines"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # The cop is config-less from the Rust side — `bundle_args` returns
        # an empty pair so it contributes nothing to nums / lists. Kept for
        # the 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
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
            Dispatch.offenses_for(processed_source, config, :leading_empty_lines)
          else
            Shirobai.check_leading_empty_lines(processed_source.buffer.source)
          end
        end

        # Eligible only when the parser-normalized buffer source is byte-
        # identical to the raw source the bundle scans. When they differ
        # (CRLF or BOM), the standalone path scans `buffer.source` so every
        # offset lines up with parser-gem's character index. Memoized per
        # investigation.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
