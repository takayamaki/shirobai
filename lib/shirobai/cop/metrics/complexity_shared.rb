# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Both complexity cops (`CyclomaticComplexity`, `PerceivedComplexity`) need
      # the same per-method analysis, so they share one bundle slot
      # (`:complexity`): Rust computes both scores per method once and each cop
      # selects its own metric. The per-file memoization lives in
      # `Shirobai::Dispatch`.
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
          # Packed args for the bundled run: `[max_cyclomatic, max_perceived]`,
          # the prefilter thresholds `Shirobai.check_complexity` receives.
          def bundle_args(config)
            [prefilter_max(config, CYCLOMATIC_BADGE), prefilter_max(config, PERCEIVED_BADGE)]
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
