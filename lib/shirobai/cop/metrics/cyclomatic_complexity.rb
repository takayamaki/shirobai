# frozen_string_literal: true

require_relative "complexity_base"

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/CyclomaticComplexity`.
      class CyclomaticComplexity < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        include ComplexityBase
        extend RuboCop::ExcludeLimit

        MSG = "Cyclomatic complexity for `%<method>s` is too high. [%<complexity>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/CyclomaticComplexity"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/CyclomaticComplexity")

        private

        def metric_score(cyclomatic, _perceived)
          cyclomatic
        end
      end
    end
  end
end
