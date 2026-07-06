# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Shared harness for the four `Metadata`-mixin cops (`MetadataStyle`,
      # `DuplicatedMetadata`, `EmptyMetadata`, `SortMetadata`).
      #
      # Rust identifies the metadata-anchor BLOCK ranges once on the shared walk
      # (direct example/group/hook blocks plus `RSpec.configure` blocks). This
      # module relocates each parser block node and runs stock's
      # `RuboCop::Cop::RSpec::Metadata#on_block` logic VERBATIM on the real
      # parser AST — so block-kind / receiver / arity filtering and the
      # `metadata_in_block` search happen exactly as stock does them, and each
      # cop's `on_metadata(symbols, hash)` sees identical `(symbols, hash)`.
      #
      # The stock entry method is renamed (`process_metadata_block`) so the
      # Commissioner never dispatches a per-node `on_block`; only
      # `on_new_investigation` is a real callback.
      module MetadataSupport
        extend RuboCop::AST::NodePattern::Macros

        # The node-pattern matchers below reference `Examples` / `ExampleGroups`
        # / `Hooks` / `#rspec?`; they resolve through this module's ancestry
        # (stock's `Metadata` mixin includes Language for the same reason).
        include RuboCop::RSpec::Language
        include Shirobai::Cop::BundleEligible

        # --- stock RuboCop::Cop::RSpec::Metadata matchers, copied verbatim ---

        # @!method rspec_metadata(node)
        def_node_matcher :rspec_metadata, <<~PATTERN
          (block
            (send
              #rspec? {#Examples.all #ExampleGroups.all #SharedGroups.all #Hooks.all} _ $...)
            ...)
        PATTERN

        # @!method rspec_configure(node)
        def_node_matcher :rspec_configure, <<~PATTERN
          (block (send #rspec? :configure) (args (arg $_)) ...)
        PATTERN

        # @!method metadata_in_block(node)
        def_node_search :metadata_in_block, <<~PATTERN
          (send (lvar %) #Hooks.all _ $...)
        PATTERN

        def on_new_investigation
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]
          ranges = resolved_anchors
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::RSpec::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            process_metadata_block(node) if node
          end
        end

        def on_metadata(_symbols, _hash)
          raise ::NotImplementedError
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte; a gated-off file (nil from Dispatch) falls back to the standalone
        # entry scanning `buffer.source`.
        def resolved_anchors
          if bundle_eligible?
            anchors = Dispatch.offenses_for(processed_source, config, metadata_slot)
            return anchors unless anchors.nil?
          end
          Shirobai.check_rspec_metadata_anchors(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # Stock `Metadata#on_block`, renamed so it is not a node callback.
        def process_metadata_block(node)
          rspec_configure(node) do |block_var|
            metadata_in_block(node, block_var) do |metadata_arguments|
              on_metadata_arguments(metadata_arguments)
            end
          end

          rspec_metadata(node) do |metadata_arguments|
            on_metadata_arguments(metadata_arguments)
          end
        end

        # Stock `Metadata#on_metadata_arguments`, copied verbatim.
        def on_metadata_arguments(metadata_arguments)
          if metadata_arguments.last&.hash_type?
            *metadata_arguments, hash = metadata_arguments
            on_metadata(metadata_arguments, hash)
          else
            on_metadata(metadata_arguments, nil)
          end
        end
      end
    end
  end
end
