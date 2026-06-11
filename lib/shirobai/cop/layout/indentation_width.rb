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
      # division of labour as the other indentation cops. Offenses come from the
      # per-file bundled run (`Shirobai::Dispatch`) while `bundle_eligible?`
      # holds; otherwise the standalone call carries the per-investigation
      # state (allowed lines / accumulated correction ranges).
      class IndentationWidth < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/IndentationWidth"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/IndentationWidth")

        # Packed args for the bundled run: the 7-element config vector
        # `Shirobai.check_indentation_width` receives (width / align-with /
        # access-modifier outdent / indented internal methods / end alignment /
        # def-end alignment / tabs).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          end_config = config.for_cop("Layout/EndAlignment")
          end_align = case end_config["EnforcedStyleAlignWith"] || "keyword"
                      when "variable" then 1
                      when "start_of_line" then 2
                      else 0
                      end
          def_end_config = config.for_cop("Layout/DefEndAlignment")
          [
            cop_config["Width"] || 2,
            cop_config["EnforcedStyleAlignWith"] == "relative_to_receiver" ? 1 : 0,
            config.for_cop("Layout/AccessModifierIndentation")["EnforcedStyle"] == "outdent" ? 1 : 0,
            config.for_cop("Layout/IndentationConsistency")["EnforcedStyle"] == "indented_internal_methods" ? 1 : 0,
            end_align,
            (def_end_config["EnforcedStyleAlignWith"] || "start_of_line") == "def" ? 1 : 0,
            (config.for_cop("Layout/IndentationStyle")["EnforcedStyle"] || "spaces") == "tabs" ? 1 : 0
          ]
        end

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
          @offense_ranges ||= []
          return Dispatch.offenses_for(processed_source, config, :indentation_width) if bundle_eligible?

          source = processed_source.raw_source
          Shirobai.check_indentation_width(
            source, bundle_args, allowed_line_numbers(source), @offense_ranges
          )
        end

        # The bundle computes this cop with empty `allowed_lines` /
        # `prior_ranges`, so it only matches the direct call while no
        # `AllowedPatterns` are configured and no correction ranges have
        # accumulated on this instance (i.e. the first / lint pass); autocorrect
        # re-passes go through the standalone entry point.
        def bundle_eligible?
          allowed_patterns.empty? && @offense_ranges.empty?
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

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
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
