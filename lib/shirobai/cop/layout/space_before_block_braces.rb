# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeBlockBraces`.
      #
      # Rust walks the AST once and reports every brace block (`do ... end`
      # already excluded) as `[left_start, left_end, space_begin, empty,
      # multiline]` in document order: `space_begin` is the begin of
      # `range_with_surrounding_space(left_brace)` and `multiline` mirrors
      # `BlockNode#multiline?` (brace lines, not the whole node).
      #
      # Everything stateful stays here, verbatim from stock: the
      # `Style/BlockDelimiters` conflict skip, `check_empty` /
      # `check_non_empty` with their `config_to_allow_offenses` bookkeeping
      # (the vendor spec asserts it), the lazy
      # `EnforcedStyleForEmptyBraces` validation raise, and the
      # `range.source`-keyed corrector.
      class SpaceBeforeBlockBraces < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MISSING_MSG = "Space missing to the left of {."
        DETECTED_MSG = "Space detected to the left of {."

        # Stock declares the same incompatibility: both cops can rewrite the
        # same brace, so RuboCop's correction loop defers a conflicting pass.
        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::SymbolProc]
        end

        def self.cop_name = "Layout/SpaceBeforeBlockBraces"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # The Rust side is config-free (every style decision happens here), so
        # nothing joins the packed bundle config.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          @buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          records_for_source.each do |left_start, left_end, space_begin, empty, multiline|
            next if conflict_with_block_delimiters?(multiline)

            left_brace = range_between(off[left_start], off[left_end])
            space_start = off[space_begin]
            used_style = space_begin < left_start ? :space : :no_space

            if empty
              check_empty(left_brace, space_start, used_style)
            else
              check_non_empty(left_brace, space_start, used_style)
            end
          end
        end

        private

        def range_between(start_pos, end_pos)
          Parser::Source::Range.new(@buffer, start_pos, end_pos)
        end

        def check_empty(left_brace, space_start, used_style)
          if style_for_empty_braces == used_style
            handle_different_styles_for_empty_braces(used_style)
            return
          elsif !config_to_allow_offenses.key?("Enabled")
            config_to_allow_offenses["EnforcedStyleForEmptyBraces"] = used_style.to_s
          end

          if style_for_empty_braces == :space
            range = left_brace
            msg = MISSING_MSG
          else
            range = range_between(space_start, left_brace.begin_pos)
            msg = DETECTED_MSG
          end
          add_offense(range, message: msg) { |corrector| autocorrect(corrector, range) }
        end

        def handle_different_styles_for_empty_braces(used_style)
          if config_to_allow_offenses["EnforcedStyleForEmptyBraces"] &&
             config_to_allow_offenses["EnforcedStyleForEmptyBraces"].to_sym != used_style
            config_to_allow_offenses.clear
            config_to_allow_offenses["Enabled"] = false
          end
        end

        def check_non_empty(left_brace, space_start, used_style)
          case used_style
          when style then correct_style_detected
          when :space then space_detected(left_brace, space_start)
          else space_missing(left_brace)
          end
        end

        def space_missing(left_brace)
          add_offense(left_brace, message: MISSING_MSG) do |corrector|
            autocorrect(corrector, left_brace)
            opposite_style_detected
          end
        end

        def space_detected(left_brace, space_start)
          space = range_between(space_start, left_brace.begin_pos)

          add_offense(space, message: DETECTED_MSG) do |corrector|
            autocorrect(corrector, space)
            opposite_style_detected
          end
        end

        def autocorrect(corrector, range)
          case range.source
          when /\s/ then corrector.remove(range)
          else corrector.insert_before(range, " ")
          end
        end

        def style_for_empty_braces
          case cop_config["EnforcedStyleForEmptyBraces"]
          when "space" then :space
          when "no_space" then :no_space
          when nil then style
          else raise "Unknown EnforcedStyleForEmptyBraces selected!"
          end
        end

        def conflict_with_block_delimiters?(multiline)
          block_delimiters_style == "line_count_based" && style == :no_space && multiline
        end

        def block_delimiters_style
          config.for_cop("Style/BlockDelimiters")["EnforcedStyle"]
        end

        def records_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_before_block_braces)
          else
            Shirobai.check_space_before_block_braces(processed_source.raw_source)
          end
        end

        # No per-investigation state on the Rust side at all, so this cop is
        # always bundle eligible. Kept for symmetry / a future fallback.
        def bundle_eligible?
          true
        end
      end
    end
  end
end
