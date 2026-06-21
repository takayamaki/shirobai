# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLines`.
      #
      # Stock's `on_new_investigation` walks `processed_source.tokens` and yields a
      # 1-byte `source_range(buffer, line, 0)` for every line `L` whose previous
      # and current text lines are both empty AND that falls inside a gap of
      # `cur_token_line - prev_token_line > 2`. The corrector removes that 1-byte
      # range (the `\n` at column 0 of the offending line).
      #
      # The Rust side reconstructs "lines with parser-gem tokens" from the prism
      # AST plus comments (see `crates/shirobai-core/src/rules/empty_lines.rs` for
      # the equivalence rules — string-content nodes fill their span, container
      # nodes mark only open/close lines, etc.) and runs the same gap-scan stock
      # does. It returns the offense byte ranges; the wrapper turns each into a
      # `Parser::Source::Range`, attaches the same MSG, and removes the range.
      #
      # Bundle-eligible only when the parser-normalized `buffer.source` is
      # byte-identical to `raw_source` (no CRLF / BOM normalization). On CRLF or
      # BOM the standalone entry point scans `buffer.source` directly so every
      # returned offset still indexes into the parser-gem buffer.
      class EmptyLines < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Extra blank line detected."

        def self.cop_name = "Layout/EmptyLines"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # The cop is config-less from the Rust side — bundle_args returns an
        # empty pair so it contributes nothing to nums / lists. Kept for the
        # 4+1 single-source-of-config convention.
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
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.remove(range)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :empty_lines)
          else
            Shirobai.check_empty_lines(processed_source.buffer.source)
          end
        end

        # Eligible only when the parser-normalized buffer source is byte-identical
        # to the raw source the bundle scans. When they differ (CRLF or BOM),
        # the standalone path scans `buffer.source` so every offset lines up
        # with parser-gem's character index. Memoized per investigation.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
