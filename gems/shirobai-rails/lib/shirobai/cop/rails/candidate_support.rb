# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Shared harness for the Architecture-B Rails cops
      # (`HttpPositionalArguments`, `DeprecatedActiveModelErrorsMethods`).
      #
      # Rust identifies candidate SEND ranges once on the shared walk; this
      # module relocates each parser send node (`Shirobai::NodeLocator`) and
      # hands it to the cop's `investigate_send`, which is stock's `on_send`
      # copied verbatim (renamed so the Commissioner never dispatches a
      # per-node `on_send`). Every guard, matcher, receiver walk and
      # autocorrect runs on the real parser AST, so detection and `-A` bytes
      # match stock exactly; the Rust prefilter only narrows which nodes are
      # visited.
      #
      # The candidate set is a superset — false positives (e.g. a routing-block
      # `get`, a non-model bare `errors`) are dropped by the stock matchers the
      # wrapper re-runs.
      module CandidateSupport
        def on_new_investigation
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            investigate_send(node) if node&.send_type?
          end
        end

        private

        # The shared walk scans `buffer.source`. A gated-off origin (nil from
        # Dispatch) would fall back to the standalone entry over
        # `buffer.source`; the rails origin has no per-file gate, so the shared
        # walk always wins.
        def resolved_candidates
          ranges = Dispatch.offenses_for(processed_source, config, candidate_slot)
          return ranges unless ranges.nil?
          fallback_candidates
        end
      end
    end
  end
end
