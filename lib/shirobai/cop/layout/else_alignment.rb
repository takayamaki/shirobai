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
      # The autocorrect is stock's own `AlignmentCorrector.correct` on the bare
      # keyword range, exactly as stock calls it: the edit anchors at the
      # keyword (not at its line start), it is skipped under
      # `Layout/IndentationStyle: tabs`, and since rubocop 1.92 a range argument
      # makes every string body in the file taboo, so an `else` inside a
      # multiline interpolation is reported as unsupported and not moved.
      #
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # behaviour is purely config-driven, so this cop is always bundle eligible.
      class ElseAlignment < RuboCop::Cop::Base
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

          records_for_source.each do |else_start, else_end, message, column_delta|
            else_range = Parser::Source::Range.new(buffer, off[else_start], off[else_end])
            add_offense(else_range, message: message) do |corrector|
              RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, else_range, column_delta)
            end
          end
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :else_alignment)
        end
      end
    end
  end
end
