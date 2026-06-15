# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/AccessModifierIndentation`.
      #
      # Rust walks the AST once, reproducing stock's `on_class` / `on_sclass` /
      # `on_module` / `on_block` alias chain: for every class-like / block
      # node, look at the body and — when it is a parser-`(begin ...)` (>= 2
      # statements; prism's single-statement `StatementsNode` mirrors stock's
      # bare body so we skip it) — inspect each direct child `send` that is a
      # `bare_access_modifier?` (no receiver / no arguments / no block, name
      # in `{public, protected, private, module_function}`). Same-line
      # modifiers (`same_line?(node, modifier)`) are skipped, exactly like
      # stock. For each kept modifier Rust returns `column_delta = expected -
      # actual` (matching stock's `@column_delta` in
      # `column_offset_between(modifier.source_range, node.loc.end)`); the
      # wrapper either fires `correct_style_detected` (delta zero) or registers
      # the offense and shifts the modifier's line by `column_delta` (mirroring
      # stock's `AlignmentCorrector.correct(corrector, processed_source, node,
      # column_delta)` for a one-line node, which simply inserts or removes
      # leading whitespace).
      #
      # Stock's `AlignmentCorrector.correct` reads `processed_source.config`
      # (for `using_tabs?`); a faithful drop-in does the same — autocorrect is
      # skipped entirely under `Layout/IndentationStyle: tabs`, just like
      # stock.
      #
      # Offenses come from the per-file bundled run (`Shirobai::Dispatch`); the
      # behaviour is purely config-driven (`EnforcedStyle` + a per-cop or
      # global `IndentationWidth`), so this cop is always bundle eligible.
      class AccessModifierIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = { indent: 0, outdent: 1 }.freeze

        def self.cop_name = "Layout/AccessModifierIndentation"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style, indentation_width]`.
        # `indentation_width` mirrors `Alignment#configured_indentation_width`
        # — the cop's own `IndentationWidth` if set, else
        # `Layout/IndentationWidth`'s `Width`, else 2.
        def self.bundle_args(config)
          own = config.for_badge(badge)
          style = STYLE_TO_U8.fetch((own["EnforcedStyle"] || "indent").to_sym, 0)
          width = own["IndentationWidth"] ||
                  config.for_cop("Layout/IndentationWidth")["Width"] ||
                  2
          [style, width]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          records_for_source.each do |start, fin, message, column_delta|
            send_range = Parser::Source::Range.new(buffer, off[start], off[fin])
            if column_delta.zero?
              correct_style_detected
              next
            end
            add_offense(send_range, message: message) do |corrector|
              # Delegate to stock's `AlignmentCorrector.correct`. It reads
              # `processed_source.config` for tab-style detection and string /
              # block-comment range avoidance; for a single-line modifier
              # those checks fall through to a single `each_line` iteration
              # that just inserts or removes leading whitespace. Passing the
              # `Parser::Source::Range` directly avoids needing the actual
              # AST node — stock's implementation accepts either (`node
              # .respond_to?(:loc) ? node.source_range : node`).
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, send_range, column_delta
              )
              opposite_or_unrecognized_style(column_delta)
            end
          end
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :access_modifier_indentation)
        end

        # Mirror stock's `check_modifier` after `add_offense`: if the actual
        # offset equals `unexpected_indent_offset` (`indentation_width -
        # expected`), call `opposite_style_detected`; otherwise
        # `unrecognized_style_detected`. Recover the actual offset from
        # `column_delta` since `column_delta = expected - actual`.
        def opposite_or_unrecognized_style(column_delta)
          expected = expected_indent_offset
          actual = expected - column_delta
          unexpected = configured_indentation_width - expected
          if actual == unexpected
            opposite_style_detected
          else
            unrecognized_style_detected
          end
        end

        def expected_indent_offset
          style == :outdent ? 0 : configured_indentation_width
        end

        # Mirror `Alignment#configured_indentation_width` exactly.
        def configured_indentation_width
          cop_config["IndentationWidth"] ||
            config.for_cop("Layout/IndentationWidth")["Width"] ||
            2
        end
      end
    end
  end
end
