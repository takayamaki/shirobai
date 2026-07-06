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
        include Shirobai::Cop::BundleEligible

        def on_new_investigation
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            investigate_send(node) if node&.send_type?
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte; a gated-off file (nil from Dispatch) falls back to the
        # standalone entry scanning `buffer.source`. The rails origin has no
        # per-file gate, so on a bundle-eligible file the bundle path always
        # wins.
        def resolved_candidates
          if bundle_eligible?
            ranges = Dispatch.offenses_for(processed_source, config, candidate_slot)
            return ranges unless ranges.nil?
          end
          fallback_candidates
        end
      end
    end
  end
end
