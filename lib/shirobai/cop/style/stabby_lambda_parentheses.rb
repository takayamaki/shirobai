# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/StabbyLambdaParentheses`.
      #
      # Rust walks the AST once and emits one record per stabby-lambda
      # `LambdaNode` whose `parameters` is a `BlockParametersNode` with
      # a non-nil inner `parameters` AND whose `()` presence disagrees
      # with the configured `EnforcedStyle`. This mirrors stock's
      # `on_send` guard (`lambda_literal? && block_node.arguments?`) plus
      # the `redundant_parentheses?` / `missing_parentheses?` style
      # comparison.
      #
      # For each emitted offense the wrapper reproduces stock's `add_offense`
      # on the `arguments` source range (`args.loc.expression`) and, depending
      # on the style, wraps with `(` `)` (require_parentheses, missing) or
      # replaces `(` with `''` + removes `)` (require_no_parentheses,
      # redundant). The behaviour is purely config-driven, so this cop is
      # always bundle-eligible.
      class StabbyLambdaParentheses < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLE_TO_U8 = {
          require_parentheses: 0,
          require_no_parentheses: 1
        }.freeze

        def self.cop_name = "Style/StabbyLambdaParentheses"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`.
        def self.bundle_args(config)
          own = config.for_badge(badge)
          style = STYLE_TO_U8.fetch(
            (own["EnforcedStyle"] || "require_parentheses").to_sym, 0
          )
          [style]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          records_for_source.each do |start, fin, paren_open_start, paren_open_end,
                                       paren_close_start, paren_close_end, message|
            args_range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(args_range, message: message) do |corrector|
              if style == :require_parentheses
                corrector.wrap(args_range, "(", ")")
              else
                open_range = Parser::Source::Range.new(
                  buffer, off[paren_open_start], off[paren_open_end]
                )
                close_range = Parser::Source::Range.new(
                  buffer, off[paren_close_start], off[paren_close_end]
                )
                corrector.replace(open_range, "")
                corrector.remove(close_range)
              end
            end
          end
        end

        private

        def records_for_source
          Dispatch.offenses_for(processed_source, config, :stabby_lambda_parentheses)
        end
      end
    end
  end
end
