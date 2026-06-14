# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/AssignmentIndentation`.
      #
      # Rust walks the AST once and, for each assignment / setter call whose
      # operator is on a different line from the RHS, computes the RHS's
      # expected display column (`leftmost_multiple_assignment.display_column +
      # IndentationWidth`). Misaligned RHSes that begin their own line emit a
      # `[rhs_start, rhs_end, column_delta]` record, which the wrapper turns
      # into an offense at the RHS range. Autocorrect re-locates the matching
      # `Parser::AST::Node` by `rhs_start` and hands it to stock's
      # `AlignmentCorrector#correct` with the same `column_delta` (this keeps
      # the heredoc / string-literal taboo handling identical to stock).
      class AssignmentIndentation < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Indent the first line of the right-hand-side of a multi-line assignment."

        def self.cop_name = "Layout/AssignmentIndentation"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # `IndentationWidth` falls back to `Layout/IndentationWidth.Width`,
        # then to 2 — exactly stock's `configured_indentation_width`.
        def self.bundle_args(config)
          own = config.for_badge(badge)["IndentationWidth"]
          width = own || config.for_cop("Layout/IndentationWidth")["Width"] || 2
          [width]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          records_for_source.each do |rhs_start, rhs_end, column_delta|
            range = Parser::Source::Range.new(buffer, off[rhs_start], off[rhs_end])
            node = locate_rhs_node(off[rhs_start])
            add_offense(range, message: MSG) do |corrector|
              # The corrector target is the Parser::AST::Node so stock's
              # `inside_string_ranges` / `block_comment_within?` checks work;
              # if relocation fails (defensive), fall back to the range.
              ::RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, node || range, column_delta
              )
            end
          end
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :assignment_indentation)
        end

        # Locate the `Parser::AST::Node` whose source range begins at
        # `rhs_begin_pos`. Stock hands the RHS node to `AlignmentCorrector` so
        # that `inside_string_ranges` (heredoc / string literal taboos) and
        # `block_comment_within?` can inspect descendants. Offenses are rare
        # in practice, so the DFS cost is negligible per investigation.
        def locate_rhs_node(rhs_begin_pos)
          processed_source.ast&.each_node do |node|
            return node if node.respond_to?(:loc) &&
                           node.source_range &&
                           node.source_range.begin_pos == rhs_begin_pos
          end
          nil
        end
      end
    end
  end
end
