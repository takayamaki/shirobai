# frozen_string_literal: true

module Shirobai
  module Cop
    module Naming
      # Drop-in Rust reimplementation of `Naming/AsciiIdentifiers`.
      #
      # Stock iterates `processed_source.tokens` (the parser-gem token stream,
      # materialized on every file — the "toucher" cost) and flags every
      # `tIDENTIFIER` (always) and `tCONSTANT` (when `AsciiConstants`) whose text
      # has a non-ASCII byte, at the first maximal run of non-ASCII chars in the
      # token. No autocorrect.
      #
      # Only a file that HAS a non-ASCII byte can ever offend, so the Rust rule
      # fast-paths every all-ASCII file with a single `is_ascii` scan and builds
      # no token stream at all. On the rare non-ASCII file it reads prism's own
      # lex tokens and maps them to the parser-gem `tIDENTIFIER` / `tCONSTANT`
      # distinction stock tests (validated against stock over every non-ASCII
      # file in the five corpora). The wrapper converts the byte range to char
      # offsets and builds stock's exact offense with `range_between`.
      class AsciiIdentifiers < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp

        IDENTIFIER_MSG = "Use only ascii symbols in identifiers."
        CONSTANT_MSG   = "Use only ascii symbols in constants."

        def self.cop_name = "Naming/AsciiIdentifiers"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed as a single 3-state num: 0 disabled (bundle skips the lex),
        # 1 enabled with AsciiConstants off, 2 enabled with AsciiConstants on.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          return [[0]] if cop_config["Enabled"] == false

          [[cop_config.fetch("AsciiConstants", true) ? 2 : 1]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)

          resolved_result(source).each do |is_constant, start, fin|
            range = range_between(off[start], off[fin])
            add_offense(range, message: is_constant ? CONSTANT_MSG : IDENTIFIER_MSG)
          end
        end

        private

        def resolved_result(source)
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :ascii_identifiers) || []
          else
            Shirobai.check_ascii_identifiers(source, cop_config["AsciiConstants"] ? true : false)
          end
        end
      end
    end
  end
end
