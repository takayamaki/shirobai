# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAroundMethodCallOperator`.
      #
      # Rust walks the AST, reproduces the stock cop's `on_send` / `on_csend`
      # (the space before/after a `.`/`&.` call operator) and `on_const` (the
      # space after a `::`), flagging a non-empty run of only spaces/tabs
      # (stock's `SPACES_REGEXP = /\A[ \t]+\z/`, so a run spanning a newline is
      # left alone). It returns, per offense, the offending whitespace range —
      # which is both the offense highlight and the autocorrect removal range
      # (stock's `corrector.remove(range)`).
      #
      # The cop has no configuration, so it is always bundle eligible; the
      # offenses come from the per-file bundled run (`Shirobai::Dispatch`). The
      # autocorrect re-passes re-investigate a fresh `ProcessedSource`, which
      # recomputes the bundle from scratch, so this cop keeps no cross-pass state.
      class SpaceAroundMethodCallOperator < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Avoid using spaces around a method call operator."

        def self.cop_name = "Layout/SpaceAroundMethodCallOperator"
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
          Dispatch.offenses_for(processed_source, config, :space_around_method_call_operator)
        end
      end
    end
  end
end
