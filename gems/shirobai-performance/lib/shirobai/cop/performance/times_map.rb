# frozen_string_literal: true

module Shirobai
  module Cop
    module Performance
      # Drop-in Rust reimplementation of `Performance/TimesMap`
      # (rubocop-performance 1.26.1).
      #
      # Rust replicates the stock `times_map_call` pattern (literal block on
      # `x.times.map` / `x.times.collect`, or a sole block-pass argument)
      # with the `handleable_receiver?` gate (int/float literal receiver of
      # `times`, or an explicit `.`-dispatched `times` call) and builds both
      # the message (`only if ... 0 or more` for non-literal counts) and the
      # `Array.new(...)` replacement from source slices. The wrapper applies
      # the replacement to the parser-send range (the call without its
      # literal block), exactly like stock's
      # `corrector.replace(map_or_collect, ...)`.
      class TimesMap < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Performance/TimesMap"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: nothing to pack (the shared segment only carries the
        # department enable flag for this cop).
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :perf_times_map)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, replace_start, replace_end, message, replacement|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              corrector.replace(
                Parser::Source::Range.new(buffer, off[replace_start], off[replace_end]),
                replacement
              )
            end
          end
        end
      end
    end
  end
end
