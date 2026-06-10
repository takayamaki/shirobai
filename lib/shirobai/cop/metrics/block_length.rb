# frozen_string_literal: true

module Shirobai
  module Cop
    module Metrics
      # Drop-in Rust reimplementation of `Metrics/BlockLength`.
      #
      # Rust parses the source, walks blocks, counts body lines (with comment and
      # `CountAsOne` handling) and excludes class constructors. Ruby applies the
      # cheap, config-driven `AllowedMethods` / `AllowedPatterns` filters and
      # registers offenses.
      class BlockLength < RuboCop::Cop::Base
        include RuboCop::Cop::AllowedMethods
        include RuboCop::Cop::AllowedPattern
        extend RuboCop::ExcludeLimit

        LABEL = "Block"
        MSG = "%<label>s has too many lines. [%<length>d/%<max>d]"

        exclude_limit "Max"

        def self.cop_name = "Metrics/BlockLength"
        def self.badge = RuboCop::Cop::Badge.parse("Metrics/BlockLength")

        def on_new_investigation
          # not implemented yet
        end
      end
    end
  end
end
