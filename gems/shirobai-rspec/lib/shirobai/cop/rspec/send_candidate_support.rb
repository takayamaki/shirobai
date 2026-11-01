# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Shared harness for the send-based metadata-family cops (`Focus`,
      # `PendingWithoutReason`).
      #
      # Rust identifies candidate SEND ranges once on the shared walk; this
      # module relocates each parser send node and hands it to the cop's
      # `investigate_send`, which is stock's `on_send` copied verbatim (renamed
      # so the Commissioner never dispatches a per-node `on_send`). Every guard,
      # matcher, and parent-relationship check runs on the real parser AST.
      module SendCandidateSupport
        def on_new_investigation
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::RSpec::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            investigate_send(node) if node&.send_type?
          end
        end

        private

        # The shared walk scans `buffer.source`. A gated-off file (nil from
        # Dispatch) falls back to the standalone entry, which scans
        # `buffer.source` too, so every offset lines up with parser-gem's index.
        def resolved_candidates
          ranges = Dispatch.offenses_for(processed_source, config, candidate_slot)
          return ranges unless ranges.nil?
          fallback_candidates
        end
      end
    end
  end
end
