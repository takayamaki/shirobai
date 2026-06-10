# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Both complexity cops (`CyclomaticComplexity`, `PerceivedComplexity`) need
      # the same per-method analysis. They run in the same investigation on the
      # same `ProcessedSource`, so we compute the Rust analysis once and memoize
      # it for the second cop — collapsing the re-parse cost to one per file
      # instead of one per cop.
      module ComplexityShared
        class << self
          def analyze(processed_source)
            return @result if @processed_source.equal?(processed_source)

            @processed_source = processed_source
            @result = Shirobai.check_complexity(processed_source.raw_source)
          end
        end
      end
    end
  end
end
