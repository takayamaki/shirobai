# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/NamedSubject`
      # (rubocop-rspec 3.10.2).
      #
      # Everything is computed on the shared walk. A reference is stock's
      # hard-coded `subject_usage` search (`$(send nil? :subject)` — the
      # literal name `subject`, never an alias nor `subject!`, zero
      # arguments, no block-pass). Rust reports it when the reference has a
      # plain-block example/hook ancestor (stock's `on_block` fires only for
      # plain blocks, so a numblock/itblock example never qualifies) that is
      # not enclosed by a shared example group under `IgnoreSharedExamples`,
      # and, under `EnforcedStyle: named_only`, when the nearest enclosing
      # `subject` definition is named. The offense range is the `subject`
      # selector. This cop has no autocorrect (stock does not either).
      #
      # Probed quirks live as differential specs in
      # named_subject_edge_cases_spec.rb.
      class NamedSubject < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible

        MSG = "Name your test subject if you need to reference it explicitly."

        def self.cop_name = "RSpec/NamedSubject"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style, ignore_shared_examples]`
        # (style 0 always / 1 named_only; ignore 0 / 1).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            cop_config.fetch("EnforcedStyle", "always").to_s == "named_only" ? 1 : 0,
            cop_config.fetch("IgnoreSharedExamples", true) ? 1 : 0
          ]
        end

        def on_new_investigation
          offenses = resolved_offenses
          return if offenses.empty?

          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          offenses.each do |(start, fin)|
            add_offense(Parser::Source::Range.new(buffer, off[start], off[fin]))
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte
        # for byte (CRLF/BOM break that — see Shirobai::Cop::BundleEligible);
        # a gated-off file (nil from Dispatch) also falls back. The fallback
        # scans `buffer.source` so every offset lines up with parser-gem's
        # index.
        def resolved_offenses
          if bundle_eligible?
            offenses = Dispatch.offenses_for(processed_source, config, :rspec_named_subject)
            return offenses unless offenses.nil?
          end
          Shirobai.check_rspec_named_subject(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end
      end
    end
  end
end
