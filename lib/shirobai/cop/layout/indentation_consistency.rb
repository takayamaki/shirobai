# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/IndentationConsistency`.
      #
      # Rust walks the AST, reproduces the stock cop's `on_begin` / `on_kwbegin`
      # over every multi-statement group (a parser-gem `begin` / `kwbegin`),
      # runs `check_alignment` (base column from the first child or
      # `base_column_for_normal_style`, sections split by `private` / `protected`
      # in `indented_internal_methods` style) and returns, per offending child,
      # its range, the `column_delta` and whether it is autocorrectable (false
      # when the child's range is `within?` an already-registered offense in the
      # same pass — `@current_offenses`). Ruby applies the realignment via
      # `AlignmentCorrector`, the same division of labour as the other
      # indentation cops. Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the autocorrect re-passes go through the
      # standalone entry point.
      class IndentationConsistency < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        MSG = "Inconsistent indentation detected."

        def self.cop_name = "Layout/IndentationConsistency"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/IndentationConsistency")

        # Packed args for the bundled run: `[indented_internal_methods]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [(cop_config["EnforcedStyle"] || "normal") == "indented_internal_methods" ? 1 : 0]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          offenses_for_source.each do |start, fin, column_delta, autocorrect|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # Always pass a block so the offense is correctable, matching stock:
            # the Alignment mixin's `register_offense` always hands `add_offense`
            # a block, even for the `within?` case (`autocorrect: false`). A
            # blockless `add_offense` would mark the offense uncorrectable. When
            # `autocorrect` is false (the offense is nested in an already-corrected
            # range) the block returns early, leaving an empty (no-op) corrector,
            # so the offense stays correctable but is not rewritten this pass.
            add_offense(range, message: MSG) do |corrector|
              next unless autocorrect

              node = node_at(off[start], off[fin])
              target = node || range
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, target, column_delta
              )
            end
          end
        end

        private

        # Unlike `Layout/IndentationWidth`, this cop keeps no cross-pass
        # instance state: the stock `within?` suppression reads
        # `@current_offenses`, which `Base` resets at the start of every
        # investigation. The autocorrect loop re-investigates a fresh
        # `ProcessedSource` each pass, so the bundled run (computed from scratch
        # per source) is always correct and this cop is always bundle eligible.
        def offenses_for_source
          Dispatch.offenses_for(processed_source, config, :indentation_consistency)
        end

        # The parser node whose `source_range` matches `[cs, ce)` (CHARACTER
        # offsets), so `AlignmentCorrector` can protect heredocs / string
        # interiors that a bare range would not. Falls back to `nil` (bare range)
        # when not found.
        def node_at(cs, ce)
          ast = processed_source.ast
          return nil unless ast

          found = nil
          ast.each_node do |n|
            r = n.source_range
            next unless r
            next unless r.begin_pos == cs && r.end_pos == ce

            found = n
            break
          end
          found
        end
      end
    end
  end
end
