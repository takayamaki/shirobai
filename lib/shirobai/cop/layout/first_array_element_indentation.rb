# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/FirstArrayElementIndentation`.
      #
      # Rust parses the source, walks every array literal (replicating the
      # `each_argument_node` / `ignore_node` claiming of arrays by a method
      # call's left parenthesis), decides the alignment base for the configured
      # `EnforcedStyle` (bracket / after-paren / parent hash key / start of
      # line) and returns each misindented first element and hanging right
      # bracket as an offense range plus its `column_delta` and message. Ruby
      # supplies the flattened config and applies the realignment via
      # `AlignmentCorrector` (the same division of labour as the other
      # alignment cops). Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the config derivation is purely config-driven,
      # so this cop is always bundle-eligible.
      class FirstArrayElementIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "special_inside_parentheses" => 0,
          "consistent" => 1,
          "align_brackets" => 2
        }.freeze

        def self.cop_name = "Layout/FirstArrayElementIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/FirstArrayElementIndentation")

        # Packed args for the bundled run: `[style, indentation_width,
        # enforce_fixed_indentation]`. The enforce flag replicates
        # `enforce_first_argument_with_fixed_indentation?`: the cop stands down
        # (except in `consistent` style) when `Layout/ArrayAlignment` enforces
        # `with_fixed_indentation`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          array_alignment_config = config.for_enabled_cop("Layout/ArrayAlignment")
          [
            STYLES.fetch(cop_config["EnforcedStyle"], 0),
            cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2,
            array_alignment_config["EnforcedStyle"] == "with_fixed_indentation"
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :first_array_element_indentation)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, column_delta, message|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # Stock yields the corrector block for every offense (no
            # per-offense gating), passing the first element NODE — so
            # `AlignmentCorrector` skips lines inside its string literals /
            # heredocs (`inside_string_ranges`) — but the right bracket RANGE.
            # Resolve the node by exact range to keep multiline-element
            # corrections identical; a `]` range never coincides with a node
            # and falls through to the range, exactly like stock.
            target = node_for(range) || range
            add_offense(range, message: message) do |corrector|
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, target, column_delta
              )
            end
          end
        end

        private

        # The outermost AST node whose source range is exactly `range`
        # (pre-order, so the array element wins over any same-range
        # descendant), or nil when no node matches. Skip nodes without a
        # source range — implicit `begin`/`mlhs` wrappers and synthetic nodes
        # built by parser-gem have `source_range == nil`.
        def node_for(range)
          root = processed_source.ast
          return nil unless root

          root.each_node.find do |node|
            sr = node.source_range
            sr && sr.begin_pos == range.begin_pos && sr.end_pos == range.end_pos
          end
        end
      end
    end
  end
end
