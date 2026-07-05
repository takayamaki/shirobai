# frozen_string_literal: true

module Shirobai
  module Cop
    module Performance
      # Drop-in Rust reimplementation of `Performance/StringInclude`
      # (rubocop-performance 1.26.1).
      #
      # Rust replicates the stock pattern union in document order (regexp
      # argument first, then regexp receiver; `!~` and `&.` only match on
      # the argument side) and the `Util::LITERAL_REGEX` literal-only gate
      # over the raw pattern source. The wrapper rebuilds the replacement
      # exactly like stock — with RuboCop's own `interpret_string_escapes`
      # and `to_string_literal` helpers (available through
      # `RuboCop::Cop::Base` -> `include Util`) — so escape interpretation
      # and quote selection cannot drift from stock byte behavior.
      class StringInclude < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Use `%<negation>sString#include?` instead of a regex match " \
              "with literal-only pattern."

        def self.cop_name = "Performance/StringInclude"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: nothing to pack (the shared segment only carries the
        # department enable flag for this cop).
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin, negation, recv_start, recv_end, dot, content|
            negation = negation == 1
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            message = format(MSG, negation: (negation ? "!" : ""))
            add_offense(range, message: message) do |corrector|
              receiver_source =
                Parser::Source::Range.new(buffer, off[recv_start], off[recv_end]).source
              literal = to_string_literal(interpret_string_escapes(content))
              new_source =
                "#{'!' if negation}#{receiver_source}#{dot}include?(#{literal})"
              corrector.replace(range, new_source)
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :perf_string_include)
          else
            Shirobai.check_perf_string_include(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
