# frozen_string_literal: true

require_relative "complexity_base"

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/PerceivedComplexity`.
      class PerceivedComplexity < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        include ComplexityBase
        extend RuboCop::ExcludeLimit

        MSG = "Perceived complexity for `%<method>s` is too high. [%<complexity>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/PerceivedComplexity"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/PerceivedComplexity")

        private

        def metric_score(_cyclomatic, perceived)
          perceived
        end
      end
    end
  end
end
