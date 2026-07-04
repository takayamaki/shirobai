# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAfterComma`.
      #
      # Stock walks `tokens` pairwise (`SpaceAfterPunctuation`); Rust
      # reconstructs the same facts byte-side: a comma token is a `,` byte
      # outside opaque literal regions, and "the next token is adjacent" is
      # "the next byte is not whitespace and not a `\`-newline continuation".
      # The allowed next tokens (`tRPAREN` / `tRBRACK` / `tPIPE` /
      # `tSTRING_DEND`, plus `tRCURLY` when
      # `Layout/SpaceInsideHashLiteralBraces` is `no_space`, plus a following
      # semicolon which has no `kind`) reduce to byte tests against the
      # interpolation-closing positions collected in the shared walk.
      #
      # Each offense is the `[start, end)` range of the comma itself; the
      # corrector replaces it with `", "`
      # (`PunctuationCorrector.add_space`).
      class SpaceAfterComma < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space missing after comma."

        def self.cop_name = "Layout/SpaceAfterComma"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run:
        # `[Layout/SpaceInsideHashLiteralBraces EnforcedStyle == 'no_space']`
        # (`nil` falls back to `'space'`, like stock's
        # `space_style_before_rcurly`).
        def self.bundle_args(config)
          style = config.for_cop("Layout/SpaceInsideHashLiteralBraces")["EnforcedStyle"] || "space"
          [[style == "no_space" ? 1 : 0], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.replace(range, "#{range.source} ")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_after_comma)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_after_comma(processed_source.buffer.source, nums[0] == 1)
          end
        end

        # See `SpaceBeforeComma#bundle_eligible?`.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
