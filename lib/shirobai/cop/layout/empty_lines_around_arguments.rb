# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLinesAroundArguments`.
      #
      # Rust walks the AST, reproduces the stock cop's `on_send` / `on_csend`
      # over every multi-line call (with at least one argument whose receiver and
      # selector share a line), scans each argument's `source_range.begin` and
      # the closing `)`/`]` for a preceding run of whitespace spanning a full
      # empty line, and returns, per offense, the offense line range. Ruby
      # reports the range and removes it, exactly like stock's
      # `corrector.remove(range)`.
      #
      # The cop has no configuration, so it is always bundle eligible; the
      # offenses come from the per-file bundled run (`Shirobai::Dispatch`). The
      # autocorrect re-passes re-investigate a fresh `ProcessedSource`, which
      # recomputes the bundle from scratch, so this cop keeps no cross-pass state.
      class EmptyLinesAroundArguments < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Empty line detected around arguments."

        def self.cop_name = "Layout/EmptyLinesAroundArguments"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No packed configuration: the cop contributes nothing to the bundle's
        # `(nums, lists)` wire format. Kept for symmetry with the other wrappers.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          offenses_for_source.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range) do |corrector|
              corrector.remove(range)
            end
          end
        end

        private

        def offenses_for_source
          Dispatch.offenses_for(processed_source, config, :empty_lines_around_arguments)
        end
      end
    end
  end
end
