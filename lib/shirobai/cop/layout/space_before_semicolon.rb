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
      # left curlies; a `"#{ ;…}"` `tSTRING_DBEG` does not).
      #
      # Each offense is the `[start, end)` whitespace run before the
      # semicolon; the corrector removes it.
      class SpaceBeforeSemicolon < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space found before semicolon."

        def self.cop_name = "Layout/SpaceBeforeSemicolon"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run:
        # `[Layout/SpaceInsideBlockBraces EnforcedStyle == 'space']`.
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
            Dispatch.offenses_for(processed_source, config, :space_before_semicolon)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_before_semicolon(processed_source.buffer.source, nums[0] == 1)
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
