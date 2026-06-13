# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/BlockAlignment`.
      #
      # Rust walks the AST once, reproducing stock's `on_block` (and
      # `on_numblock` / `on_itblock`): it picks the block's alignment target by
      # walking the lineage (`block_end_align_target`), unwraps op_asgn / masgn
      # LHS (`find_lhs_node`), and decides whether the closing token (`end` /
      # `}`) is misaligned under the configured `EnforcedStyleAlignWith`. Each
      # misaligned block returns its closing-token range, the formatted message
      # (including the `either`-only " or ..." alternative), and the autocorrect
      # target column (`compute_start_col`).
      #
      # The autocorrect mirrors `BlockAlignment#autocorrect`: the delta between
      # the target column and the closing token's column is applied as inserted
      # spaces before the token (positive delta) or removed leading whitespace
      # (negative delta).
      #
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # style is purely config-driven, so this cop is always bundle eligible.
      class BlockAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { either: 0, start_of_block: 1, start_of_line: 2 }.freeze

        def self.cop_name = "Layout/BlockAlignment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`. `EnforcedStyleAlignWith`
        # defaults to `either` (0) when the config does not mention this cop.
        def self.bundle_args(config)
          align = config.for_badge(badge)["EnforcedStyleAlignWith"]
          [STYLE_TO_U8.fetch((align || "either").to_sym, 0)]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          records_for_source.each do |end_start, end_end, message, align_column|
            end_range = Parser::Source::Range.new(buffer, off[end_start], off[end_end])
            add_offense(end_range, message: message) do |corrector|
              autocorrect(corrector, buffer, end_range, align_column)
            end
          end
        end

        # `EnforcedStyleAlignWith` keys the style.
        def style_parameter_name
          "EnforcedStyleAlignWith"
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :block_alignment)
        end

        # `BlockAlignment#autocorrect`: `delta = start_col - loc_end.column`.
        # Positive inserts `delta` spaces before the closing token; negative
        # removes `-delta` characters of leading whitespace.
        def autocorrect(corrector, buffer, end_range, start_col)
          delta = start_col - end_range.column
          if delta.positive?
            corrector.insert_before(end_range, " " * delta)
          elsif delta.negative?
            range = Parser::Source::Range.new(buffer, end_range.begin_pos + delta, end_range.begin_pos)
            corrector.remove(range)
          end
        end
      end
    end
  end
end
