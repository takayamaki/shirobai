# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/DefEndAlignment`.
      #
      # Rust walks the AST once, reproducing stock's callback dispatch: `on_def`
      # / `on_defs` check a bare definition's `end` against the configured style
      # (the `def` keyword), while `on_send` handles a `def_modifier?` send
      # (`private def foo`) — there both styles are available (`def` = the def
      # keyword, `start_of_line` = the `private def` prefix) and the inner def is
      # `ignore_node`d so its own callback is a no-op. Per checked `end` it
      # returns the matched styles (for `config_to_allow_offenses` parity) and,
      # when the configured style is not matched, the offense message and the
      # autocorrect target column.
      #
      # This wrapper replays stock's `check_end_kw_alignment` decisions in walk
      # order: `correct_style_detected` when the configured style already
      # matches, otherwise `add_offense` plus `style_detected(matched styles)`.
      # The autocorrect mirrors `AlignmentCorrector#align_end`: replace the
      # whitespace before `end` with the target indentation, or (when something
      # non-space precedes `end`) insert a newline + indentation after it. The
      # alignment column comes from Rust (it already encodes the `node` vs
      # `node.parent` anchor choice for `start_of_line`).
      #
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # style is purely config-driven, so this cop is always bundle eligible.
      class DefEndAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { start_of_line: 0, def: 1 }.freeze
        U8_TO_STYLE = STYLE_TO_U8.invert.freeze

        def self.cop_name = "Layout/DefEndAlignment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`. `EnforcedStyleAlignWith`
        # defaults to `start_of_line` (0) when the config does not mention it.
        def self.bundle_args(config)
          align = config.for_badge(badge)["EnforcedStyleAlignWith"]
          [STYLE_TO_U8.fetch((align || "start_of_line").to_sym, 0)]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          style_bit = STYLE_TO_U8.fetch(style)

          records_for_source.each do |end_start, end_end, matching, message, align_column|
            if matching.include?(style_bit)
              correct_style_detected
              next
            end

            end_range = Parser::Source::Range.new(buffer, off[end_start], off[end_end])
            add_offense(end_range, message: message) do |corrector|
              align_end(corrector, buffer, off, end_range, align_column)
            end
            style_detected(matching.map { |bit| U8_TO_STYLE.fetch(bit) })
          end
        end

        # `EndKeywordAlignment` keys its style off `EnforcedStyleAlignWith`.
        def style_parameter_name
          "EnforcedStyleAlignWith"
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :def_end_alignment)
        end

        # `AlignmentCorrector#align_end`: the whitespace run immediately before
        # `end` (`end.begin_pos - end.column .. end.begin_pos`). If it is all
        # whitespace, replace it with the target indentation; otherwise insert a
        # newline + indentation after it.
        def align_end(corrector, buffer, off, end_range, column)
          ws_begin = end_range.begin_pos - end_range.column
          whitespace = Parser::Source::Range.new(buffer, ws_begin, end_range.begin_pos)
          indentation = indentation_string(column)

          if whitespace.source.strip.empty?
            corrector.replace(whitespace, indentation)
          else
            corrector.insert_after(whitespace, "\n#{indentation}")
          end
        end

        def indentation_string(column)
          (using_tabs? ? "\t" : " ") * column
        end

        # Mirror stock `AlignmentCorrector#using_tabs?` exactly: it reads
        # `processed_source.config`, not the cop's own `config`. In a real
        # rubocop run the two are the same object (`Runner` does
        # `processed_source.config = config`), so the output is identical. The
        # distinction only matters when `processed_source.config` is `nil`
        # (e.g. a `ProcessedSource` built without a config, as the bare
        # Commissioner harness does): stock then raises `NoMethodError` inside
        # the `add_offense` corrector block and the Commissioner silently drops
        # the offense. Reading the cop's `config` instead would make this
        # drop-in *not* drop the offense there, diverging from stock by +1. A
        # faithful drop-in must read the same source stock does.
        def using_tabs?
          processed_source.config.for_cop("Layout/IndentationStyle")["EnforcedStyle"] == "tabs"
        end
      end
    end
  end
end
