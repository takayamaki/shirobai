# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/SortMetadata`
      # (rubocop-rspec 3.10.2).
      #
      # Rust supplies the shared metadata-anchor block ranges; this wrapper
      # relocates each parser block node and runs stock's `on_metadata` plus
      # autocorrect VERBATIM. The comparator (`key.source.downcase` /
      # `value.to_s.downcase`), the ambiguous-trailing guard, and the
      # `map(&:source).join(", ")` replacement all match byte for byte.
      class SortMetadata < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        include RuboCop::RSpec::Language
        include Shirobai::Cop::RSpec::MetadataSupport
        include RuboCop::Cop::RangeHelp

        MSG = "Sort metadata alphabetically."

        def self.cop_name = "RSpec/SortMetadata"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def metadata_slot = :rspec_sort_metadata

        # @!method match_ambiguous_trailing_metadata?(node)
        def_node_matcher :match_ambiguous_trailing_metadata?, <<~PATTERN
          (send _ _ _ ... !{hash sym any_str})
        PATTERN

        # --- stock's private methods, copied verbatim (rubocop-rspec 3.10.2) ---

        def on_metadata(args, hash)
          pairs = hash&.pairs || []
          symbols = trailing_symbols(args)
          return if sorted?(symbols, pairs)

          crime_scene = crime_scene(symbols, pairs)
          add_offense(crime_scene) do |corrector|
            corrector.replace(crime_scene, replacement(symbols, pairs))
          end
        end

        private

        def trailing_symbols(args)
          args = args[...-1] if last_arg_could_be_a_hash?(args)
          args.reverse.take_while(&:sym_type?).reverse
        end

        def last_arg_could_be_a_hash?(args)
          args.last && match_ambiguous_trailing_metadata?(args.last.parent)
        end

        def crime_scene(symbols, pairs)
          metadata = symbols + pairs

          range_between(
            metadata.first.source_range.begin_pos,
            metadata.last.source_range.end_pos
          )
        end

        def replacement(symbols, pairs)
          (sort_symbols(symbols) + sort_pairs(pairs)).map(&:source).join(", ")
        end

        def sorted?(symbols, pairs)
          symbols == sort_symbols(symbols) && pairs == sort_pairs(pairs)
        end

        def sort_pairs(pairs)
          pairs.sort_by { |pair| pair.key.source.downcase }
        end

        def sort_symbols(symbols)
          symbols.sort_by { |symbol| symbol.value.to_s.downcase }
        end
      end
    end
  end
end
