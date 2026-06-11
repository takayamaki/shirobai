# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/IndentationWidth`.
      #
      # Rust walks the AST, decides the base location for every indentable body
      # (def/class/module/if/case/while/for/block/rescue/ensure/begin), computes
      # `column_offset_between(body, base)` and the resulting `column_delta`, and
      # returns the offense range, the message, the `within?` autocorrect flag
      # and the node range to realign. Ruby supplies the flattened config (and
      # the `AllowedPatterns`-matched line numbers, since regex matching stays in
      # Ruby) and applies the realignment via `AlignmentCorrector`, the same
      # division of labour as the other indentation cops.
      class IndentationWidth < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/IndentationWidth"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/IndentationWidth")

        def on_new_investigation
          buffer = processed_source.buffer

          offenses_for_source.each do |start, fin, column_delta, message, autocorrect, cs, ce|
            # Mirror `other_offense_in_same_range?`: the cop instance accumulates
            # correction ranges across autocorrect iterations so a correction
            # nested in an already-corrected range is reported but not corrected.
            @offense_ranges << [cs, ce] if autocorrect

            range = Parser::Source::Range.new(buffer, start, fin)
            # Key the split on the per-offense flag, not `autocorrect?` mode: the
            # block runs in lint mode too and the non-empty corrector is what
            # keeps the offense correctable to match stock (see argument_alignment).
            unless autocorrect
              add_offense(range, message: message)
              next
            end

            add_offense(range, message: message) do |corrector|
              node = node_at(cs, ce)
              target = node || Parser::Source::Range.new(buffer, cs, ce)
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, target, column_delta
              )
            end
          end
        end

        private

        def offenses_for_source
          source = processed_source.raw_source
          @offense_ranges ||= []
          Shirobai.check_indentation_width(
            source, packed_config, allowed_line_numbers(source), @offense_ranges
          )
        end

        # The parser node whose `source_range` begins at `cs` and ends at `ce`,
        # so `AlignmentCorrector` can protect heredocs / string interiors that a
        # bare range would not. Falls back to `nil` (bare range) when not found.
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

        # Config-derived and stable for the life of the instance.
        def packed_config
          @packed_config ||= [
            configured_indentation_width,
            (cop_config["EnforcedStyleAlignWith"] == "relative_to_receiver") ? 1 : 0,
            access_modifier_indentation_style == "outdent" ? 1 : 0,
            indentation_consistency_style == "indented_internal_methods" ? 1 : 0,
            end_alignment_value,
            def_end_alignment_def? ? 1 : 0,
            using_tabs? ? 1 : 0
          ]
        end

        def configured_indentation_width
          cop_config["Width"] || 2
        end

        def end_alignment_value
          end_config = config.for_cop("Layout/EndAlignment")
          case end_config["EnforcedStyleAlignWith"] || "keyword"
          when "variable" then 1
          when "start_of_line" then 2
          else 0
          end
        end

        def def_end_alignment_def?
          def_end_config = config.for_cop("Layout/DefEndAlignment")
          (def_end_config["EnforcedStyleAlignWith"] || "start_of_line") == "def"
        end

        def access_modifier_indentation_style
          config.for_cop("Layout/AccessModifierIndentation")["EnforcedStyle"]
        end

        def indentation_consistency_style
          config.for_cop("Layout/IndentationConsistency")["EnforcedStyle"]
        end

        def indentation_style
          config.for_cop("Layout/IndentationStyle")["EnforcedStyle"] || "spaces"
        end

        def using_tabs?
          indentation_style == "tabs"
        end

        # 1-based line numbers whose content matches an `AllowedPatterns` entry.
        def allowed_line_numbers(source)
          @allowed_patterns_list ||= allowed_patterns
          return [] if @allowed_patterns_list.empty?

          source.lines.each_with_index.filter_map do |line, idx|
            (idx + 1) if matches_allowed_pattern?(line.chomp)
          end
        end
      end
    end
  end
end
