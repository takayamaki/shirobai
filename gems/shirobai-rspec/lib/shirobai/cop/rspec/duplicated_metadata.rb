# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/DuplicatedMetadata`
      # (rubocop-rspec 3.10.2).
      #
      # Rust supplies the shared metadata-anchor block ranges; this wrapper
      # relocates each parser block node and runs stock's `on_metadata` plus
      # autocorrect VERBATIM, so structural `eql?` duplicate detection and the
      # comma/space-aware removal match byte for byte.
      class DuplicatedMetadata < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::MetadataSupport
        include RuboCop::Cop::RangeHelp

        MSG = "Avoid duplicated metadata."

        def self.cop_name = "RSpec/DuplicatedMetadata"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less; the shared anchor list needs no per-cop settings.
        def self.bundle_args(_config)
          []
        end

        def metadata_slot = :rspec_duplicated_metadata

        # --- stock's private methods, copied verbatim (rubocop-rspec 3.10.2) ---

        def on_metadata(symbols, _hash)
          symbols.each do |symbol|
            on_metadata_symbol(symbol)
          end
        end

        private

        def on_metadata_symbol(node)
          return unless duplicated?(node)

          add_offense(node) do |corrector|
            autocorrect(corrector, node)
          end
        end

        def autocorrect(corrector, node)
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

        def duplicated?(node)
          node.left_siblings.any? do |sibling|
            sibling.eql?(node)
          end
        end
      end
    end
  end
end
