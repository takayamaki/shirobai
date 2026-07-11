# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/LineContinuationSpacing`.
      #
      # Stock only inspects files that contain a backslash, and its detection
      # (`find_offensive_spacing` regex per line) and its ignored-range set
      # (`ignored_literal_ranges` over `processed_source.ast` +
      # `comment_ranges` over `processed_source.comments`) never touch the token
      # stream. The one token access is `last_line`
      # (`processed_source.tokens.last.line`), which materializes the parser-gem
      # token stream on EVERY file — the "toucher" cost. Rust supplies
      # `last_line` (stock's identical definition, shared with `Layout/EndOfLine`)
      # from the shared parse, and the wrapper runs stock's own
      # `on_new_investigation` body with that value injected, so detection, the
      # offense range, and the autocorrect bytes are stock's code by
      # construction.
      #
      # `last_line` is line-based (identical whether computed from `raw_source`
      # or the CR-stripped `buffer.source`) and the offense range comes from
      # stock's own `source_range`, so no `SourceOffsets` conversion is needed.
      class LineContinuationSpacing < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Layout/LineContinuationSpacing"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less on the Rust side (`last_line` needs no config; the wrapper
        # reads `EnforcedStyle` from `cop_config`). Kept for the 4+1 convention.
        def self.bundle_args(_config)
          []
        end

        # --- stock's `on_new_investigation`, with `last_line` from Rust ---

        def on_new_investigation
          return unless processed_source.raw_source.include?("\\")

          last_line = resolved_last_line

          processed_source.raw_source.lines.each_with_index do |line, index|
            break if index >= last_line

            line_number = index + 1
            investigate(line, line_number)
          end
        end

        private

        def resolved_last_line
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :line_continuation_spacing)
          else
            # Same quantity as `Layout/EndOfLine`'s `last_line`.
            Shirobai.check_end_of_line(processed_source.buffer.source)
          end
        end

        # --- everything below is stock, verbatim ---

        def investigate(line, line_number)
          offensive_spacing = find_offensive_spacing(line)
          return unless offensive_spacing

          range = source_range(
            processed_source.buffer,
            line_number,
            line.length - offensive_spacing.length - 1,
            offensive_spacing.length
          )

          return if ignore_range?(range)

          add_offense(range) { |corrector| autocorrect(corrector, range) }
        end

        def find_offensive_spacing(line)
          if no_space_style?
            line[/\s+\\$/, 0]
          elsif space_style?
            line[/((?<!\s)|\s{2,})\\$/, 0]
          end
        end

        def message(_range)
          if no_space_style?
            "Use zero spaces in front of backslash."
          elsif space_style?
            "Use one space in front of backslash."
          end
        end

        def autocorrect(corrector, range)
          correction = if no_space_style?
                         "\\"
                       elsif space_style?
                         " \\"
                       end
          corrector.replace(range, correction)
        end

        # rubocop:disable Metrics/AbcSize, Metrics/CyclomaticComplexity, Metrics/PerceivedComplexity
        def ignored_literal_ranges(ast)
          # which lines start inside a string literal?
          return [] if ast.nil?

          ast.each_node(:str, :dstr, :array).with_object(Set.new) do |literal, ranges|
            loc = literal.location

            if literal.array_type?
              next unless literal.percent_literal?

              ranges << loc.expression
            elsif literal.heredoc?
              ranges << loc.heredoc_body
            elsif literal.loc?(:begin) || ignored_parent?(literal)
              ranges << loc.expression
            end
          end
        end
        # rubocop:enable Metrics/AbcSize, Metrics/CyclomaticComplexity, Metrics/PerceivedComplexity

        def comment_ranges(comments)
          comments.map(&:source_range)
        end

        def ignore_range?(backtick_range)
          ignored_ranges.any? { |range| range.contains?(backtick_range) }
        end

        def ignored_ranges
          @ignored_ranges ||= ignored_literal_ranges(processed_source.ast) +
                              comment_ranges(processed_source.comments)
        end

        def ignored_parent?(node)
          return false unless node.parent

          node.parent.type?(:regexp, :xstr)
        end

        def no_space_style?
          cop_config["EnforcedStyle"] == "no_space"
        end

        def space_style?
          cop_config["EnforcedStyle"] == "space"
        end
      end
    end
  end
end
