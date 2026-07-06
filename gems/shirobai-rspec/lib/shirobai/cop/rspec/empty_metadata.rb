# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/EmptyMetadata`
      # (rubocop-rspec 3.10.2).
      #
      # Rust supplies the shared metadata-anchor block ranges; this wrapper
      # relocates each parser block node and runs stock's `on_metadata` plus
      # autocorrect VERBATIM (empty-hash detection, kwsplat guard, and the
      # comma/space-aware removal all match byte for byte).
      class EmptyMetadata < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::MetadataSupport
        include RuboCop::Cop::RangeHelp

        MSG = "Avoid empty metadata hash."

        def self.cop_name = "RSpec/EmptyMetadata"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def metadata_slot = :rspec_empty_metadata

        # --- stock's private methods, copied verbatim (rubocop-rspec 3.10.2) ---

        def on_metadata(_symbols, hash)
          return unless hash&.pairs&.empty?
          return if hash.children.any?(&:kwsplat_type?)

          add_offense(hash) do |corrector|
            remove_empty_metadata(corrector, hash)
          end
        end

        private

        def remove_empty_metadata(corrector, node)
          corrector.remove(
            range_with_surrounding_comma(
              range_with_surrounding_space(
                node.source_range,
                side: :left
              ),
              :left
            )
          )
        end
      end
    end
  end
end
