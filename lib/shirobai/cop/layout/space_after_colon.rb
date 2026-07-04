# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAfterColon`.
      #
      # Stock is AST-only (`on_pair` / `on_kwoptarg`); Rust mirrors it on the
      # shared walk. A colon pair's colon is the last byte of the key
      # symbol's `closing_loc` (prism keeps the label colon inside the
      # symbol — plain, quoted and interpolated keys alike, in hashes,
      # keyword arguments and hash patterns); a keyword optional parameter's
      # colon is the last byte of its `name_loc`. Rockets and value
      # omissions (`{a:}`, `in {a:}` — an `ImplicitNode` value) are skipped,
      # and required keyword arguments (`a:`) are not kwoptargs. The byte
      # after the colon is checked against `/\s/` (EOF does not match).
      #
      # Each offense is the `[start, end)` range of the colon; the corrector
      # inserts a space after it.
      class SpaceAfterColon < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Space missing after colon."

        def self.cop_name = "Layout/SpaceAfterColon"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to `nums` / `lists`.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.insert_after(range, " ")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_after_colon)
          else
            Shirobai.check_space_after_colon(processed_source.buffer.source)
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
