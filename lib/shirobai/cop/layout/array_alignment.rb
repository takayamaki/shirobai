# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ArrayAlignment`.
      #
      # Rust parses the source, walks every 2+-element array literal (plus the
      # bracket-less arrays parser-gem synthesizes: single-assignment RHS lists
      # and `rescue` exception lists; masgn RHS lists are skipped like stock),
      # picks the alignment base for the configured `EnforcedStyle`
      # (`with_first_element` / `with_fixed_indentation`) and returns each
      # misaligned element as an offense range plus its `column_delta`. Ruby
      # supplies the flattened config and applies the realignment via
      # `AlignmentCorrector` (the same division of labour as
      # `Layout/ArgumentAlignment`). The corrector receives the parser NODE for
      # the offense range (resolved like `Layout/IndentationConsistency`), so
      # heredoc bodies and multi-line string interiors inside a shifted element
      # stay untouched, matching stock's taboo-range protection. Offenses come
      # from the per-file bundled run (`Shirobai::Dispatch`); the config
      # derivation is purely config-driven, so this cop is always
      # bundle-eligible.
      class ArrayAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        ALIGN_ELEMENTS_MSG = "Align the elements of an array literal " \
                             "if they span more than one line."

        FIXED_INDENT_MSG = "Use one level of indentation for elements " \
                           "following the first line of a multi-line array."

        def self.cop_name = "Layout/ArrayAlignment"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/ArrayAlignment")

        # Packed args for the bundled run: `[style, indentation_width]`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config["EnforcedStyle"] == "with_fixed_indentation" ? 1 : 0,
            cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          message = fixed_indentation? ? FIXED_INDENT_MSG : ALIGN_ELEMENTS_MSG

          offenses = Dispatch.offenses_for(processed_source, config, :array_alignment)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, column_delta, autocorrect|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # Always pass a block so the offense is correctable, matching stock:
            # the Alignment mixin's `register_offense` always hands `add_offense`
            # a block, even for the `within?` case (`autocorrect: false`) where it
            # passes a nil node and the corrector ends up empty. A blockless
            # `add_offense` would instead mark the offense uncorrectable. When
            # `autocorrect` is false the block returns early, leaving an empty
            # (no-op) corrector, so the offense stays correctable but is not
            # rewritten this pass.
            add_offense(range, message: message) do |corrector|
              next unless autocorrect

              # Stock passes the element NODE, whose string/heredoc interiors
              # `AlignmentCorrector` marks taboo. A bare range would realign
              # heredoc bodies inside the element.
              target = node_at(off[start], off[fin]) || range
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, target, column_delta
              )
            end
          end
        end

        private

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end

        def fixed_indentation?
          bundle_args[0] == 1
        end

        # The parser node whose `source_range` matches `[cs, ce)` (CHARACTER
        # offsets), so `AlignmentCorrector` can protect heredocs / string
        # interiors that a bare range would not. Falls back to `nil` (bare
        # range) when not found.
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
