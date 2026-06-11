# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Both complexity cops (`CyclomaticComplexity`, `PerceivedComplexity`) need
      # the same per-method analysis. They run in the same investigation on the
      # same `ProcessedSource` with the same `Config`, so we compute the Rust
      # analysis once and memoize it for the second cop — collapsing the
      # re-parse cost to one per file instead of one per cop.
      #
      # Rust only returns the methods exceeding either cop's `Max`
      # (`cyclomatic > max || perceived > max`), so the compliant majority is
      # never marshaled; each cop then re-checks its own metric exactly as
      # before. Stock RuboCop touches non-exceeding methods nowhere (the
      # `self.max =` ExcludeLimit bookkeeping only happens inside the offense
      # path), so the prefilter is unobservable.
      module ComplexityShared
        CYCLOMATIC_BADGE = RuboCop::Cop::Badge.parse("Metrics/CyclomaticComplexity")
        PERCEIVED_BADGE = RuboCop::Cop::Badge.parse("Metrics/PerceivedComplexity")

        class << self
          # Memoized per (`processed_source`, `config`) identity, following
          # `Shirobai::Dispatch`: both cop instances in one run share the same
          # config object, and the autocorrect loop builds a fresh
          # `ProcessedSource`, which naturally recomputes.
          def analyze(processed_source, config)
            unless @processed_source.equal?(processed_source) && @config.equal?(config)
              @processed_source = processed_source
              @config = config
              @result = Shirobai.check_complexity(
                processed_source.raw_source,
                prefilter_max(config, CYCLOMATIC_BADGE),
                prefilter_max(config, PERCEIVED_BADGE)
              )
            end
            @result
          end

          private

          # The cop's configured `Max`, resolved exactly like `Base#cop_config`
          # (`for_badge` merges the department config). Any non-natural value
          # (absent, negative, float, ...) falls back to 0, which disables the
          # prefilter for that metric — every method scores at least 1 — so the
          # cop's own `complexity > max` filter stays the behavioral source of
          # truth.
          def prefilter_max(config, badge)
            max = config.for_badge(badge)["Max"]
            max.is_a?(Integer) && max >= 0 ? max : 0
          end
        end
      end
    end
  end
end
