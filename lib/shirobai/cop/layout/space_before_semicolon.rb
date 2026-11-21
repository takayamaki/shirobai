# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeSemicolon`.
      #
      # Same byte-side reconstruction as `SpaceBeforeComma` (they share
      # stock's `SpaceBeforePunctuation`). The block-`{` skip is live here:
      # `loop { ; 1 }` keeps its space when `Layout/SpaceInsideBlockBraces`
      # is `space` (a lambda's `tLAMBEG` and `BEGIN`/`END` braces count as
      # left curlies; a `"#{ ;…}"` `tSTRING_DBEG` does not). Since 1.92.3 a
      # `(` (`tLPAREN` / `tLPAREN2`, not the `tLPAREN_ARG` of `foo (`) keeps
      # its space too when `Layout/SpaceInsideParens` wants one.
      #
      # Each offense is the `[start, end)` whitespace run before the
      # semicolon; the corrector removes it.
      class SpaceBeforeSemicolon < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space found before semicolon."
        SPACE_INSIDE_PARENS_STYLES = %w[space compact].freeze

        def self.cop_name = "Layout/SpaceBeforeSemicolon"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run:
        # `[bits]`: 1 = `Layout/SpaceInsideBlockBraces` `EnforcedStyle == 'space'`
        # (`space_required_after_lcurly?`), 2 = the enabled
        # `Layout/SpaceInsideParens` `EnforcedStyle` is `space` or `compact`
        # (`space_required_after_lparen?`, 1.92.3).
        def self.bundle_args(config)
          style = config.for_cop("Layout/SpaceInsideBlockBraces")["EnforcedStyle"] || "space"
          parens = config.for_enabled_cop("Layout/SpaceInsideParens")["EnforcedStyle"]
          bits = (style == "space" ? 1 : 0) | (SPACE_INSIDE_PARENS_STYLES.include?(parens) ? 2 : 0)
          [[bits], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = processed_source.buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.remove(range)
            end
          end
        end

        private

        def resolved_offenses
          Dispatch.offenses_for(processed_source, config, :space_before_semicolon)
        end
      end
    end
  end
end
