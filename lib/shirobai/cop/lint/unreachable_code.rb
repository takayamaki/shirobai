# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/UnreachableCode`.
      #
      # Detection happens entirely in Rust (no autocorrect, mirroring stock).
      # Ruby only turns byte offsets into offenses.
      #
      # The cop carries no config so `bundle_args` returns an empty vector and
      # the bundle path is always taken (the standalone entry point exists for
      # the per-cop fallback to keep symmetry with the other config-less Lint
      # cops, but is not exercised on the normal investigation path).
      class UnreachableCode < RuboCop::Cop::Base
        MSG = "Unreachable code detected."

        def self.cop_name = "Lint/UnreachableCode"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/UnreachableCode")

        # Config-less cop. Returns an empty array so `Dispatch.packed_config`
        # can splat it without touching `nums` or `lists`.
        def self.bundle_args(_config)
          []
        end

        def bundle_eligible?
          true
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          fetch_offenses.each do |start_offset, end_offset|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            add_offense(range)
          end
        end

        private

        def fetch_offenses
          Dispatch.offenses_for(processed_source, config, :unreachable_code)
        end
      end
    end
  end
end
