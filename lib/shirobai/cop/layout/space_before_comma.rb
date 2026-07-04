# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeComma`.
      #
      # Stock walks `sorted_tokens` pairwise (`SpaceBeforePunctuation`); Rust
      # reconstructs the same facts byte-side: a comma token is a `,` byte
      # outside strings / symbols / regexps / comments / heredoc bodies /
      # `$,` / `__END__` data, and the previous token's end is the first
      # non-whitespace byte to the left on the same line. A block or lambda
      # `{` right before the comma is skipped when
      # `Layout/SpaceInsideBlockBraces` wants a space after it
      # (`space_required_after_lcurly?` — unreachable for commas in code that
      # parses, but mirrored for exactness).
      #
      # Each offense is the `[start, end)` whitespace run before the comma;
      # the corrector removes it (`PunctuationCorrector.remove_space`).
      class SpaceBeforeComma < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space found before comma."

        def self.cop_name = "Layout/SpaceBeforeComma"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run:
        # `[Layout/SpaceInsideBlockBraces EnforcedStyle == 'space']`
        # (`nil` falls back to `'space'`, like stock's
        # `space_required_after_lcurly?`).
        def self.bundle_args(config)
          style = config.for_cop("Layout/SpaceInsideBlockBraces")["EnforcedStyle"] || "space"
          [[style == "space" ? 1 : 0], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
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
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_before_comma)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_before_comma(processed_source.buffer.source, nums[0] == 1)
          end
        end

        # Eligible only when the parser-normalized buffer source is
        # byte-identical to the raw source the bundle scans; on a CRLF/BOM
        # mismatch the standalone path scans `buffer.source` so offsets line
        # up with parser-gem's index. Memoized per investigation.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
