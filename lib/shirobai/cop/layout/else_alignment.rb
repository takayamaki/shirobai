# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ElseAlignment`.
      #
      # Rust walks the AST once, reproducing stock's callbacks: `on_if` /
      # `on_case` / `on_case_match` / `on_rescue` and the `CheckAssignment`
      # family. Each `else` / `elsif` keyword (and the `else` of a `case` /
      # `begin` / `def` / block `rescue` chain) is checked against the construct
      # it belongs to; `elsif` chains carry the same base down the recursion, and
      # an `if` on an assignment RHS aligns with the assignment node
      # (`variable`) or the `if` (`keyword`) per `Layout/EndAlignment`'s
      # `EnforcedStyleAlignWith`. Per misaligned keyword Rust returns the keyword
      # range, the formatted message, and the signed column delta.
      #
      # The autocorrect mirrors `AlignmentCorrector.correct` for the single-line
      # keyword range: shift the keyword's line by the column delta (insert
      # spaces at the line start for a positive delta, remove leading whitespace
      # for a negative one). As in stock, the correction is skipped (offense
      # still registered) when `Layout/IndentationStyle` is `tabs`.
      #
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # behaviour is purely config-driven, so this cop is always bundle eligible.
      class ElseAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { keyword: 0, variable: 1, start_of_line: 2 }.freeze

        def self.cop_name = "Layout/ElseAlignment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`. `ElseAlignment` reads
        # `Layout/EndAlignment`'s `EnforcedStyleAlignWith` (default `keyword`).
        def self.bundle_args(config)
          align = config.for_cop("Layout/EndAlignment")["EnforcedStyleAlignWith"]
          [STYLE_TO_U8.fetch((align || "keyword").to_sym, 0)]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          tabs = using_tabs?

          records_for_source.each do |else_start, else_end, message, column_delta|
            else_range = Parser::Source::Range.new(buffer, off[else_start], off[else_end])
            add_offense(else_range, message: message) do |corrector|
              next if tabs

              align_keyword(corrector, buffer, else_range, column_delta)
            end
          end
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :else_alignment)
        end

        # `AlignmentCorrector.correct` for a single-line range: shift the
        # keyword's line by `column_delta`. The keyword begins its line, so its
        # line start is `else_begin - else_column`; a positive delta inserts that
        # many spaces there, a negative one removes `-delta` leading characters.
        def align_keyword(corrector, buffer, else_range, column_delta)
          line_begin = else_range.begin_pos - else_range.column
          if column_delta.positive?
            corrector.insert_before(
              Parser::Source::Range.new(buffer, line_begin, line_begin),
              " " * column_delta
            )
          elsif column_delta.negative?
            range = Parser::Source::Range.new(buffer, line_begin, line_begin - column_delta)
            corrector.remove(range)
          end
        end

        def using_tabs?
          config.for_cop("Layout/IndentationStyle")["EnforcedStyle"] == "tabs"
        end
      end
    end
  end
end
