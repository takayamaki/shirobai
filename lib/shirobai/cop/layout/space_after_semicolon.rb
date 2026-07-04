# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAfterSemicolon`.
      #
      # Same byte-side reconstruction as `SpaceAfterComma` (they share
      # stock's `SpaceAfterPunctuation`), with the semicolon-specific
      # guards: a `;;` sequence is skipped (`semicolon_sequence?`) and the
      # `tRCURLY` allowance reads `Layout/SpaceInsideBlockBraces` instead of
      # the hash-brace cop.
      #
      # Each offense is the `[start, end)` range of the semicolon itself;
      # the corrector replaces it with `"; "`.
      class SpaceAfterSemicolon < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space missing after semicolon."

        def self.cop_name = "Layout/SpaceAfterSemicolon"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run:
        # `[Layout/SpaceInsideBlockBraces EnforcedStyle == 'no_space']`.
        def self.bundle_args(config)
          style = config.for_cop("Layout/SpaceInsideBlockBraces")["EnforcedStyle"] || "space"
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
            Dispatch.offenses_for(processed_source, config, :space_after_semicolon)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_after_semicolon(processed_source.buffer.source, nums[0] == 1)
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
