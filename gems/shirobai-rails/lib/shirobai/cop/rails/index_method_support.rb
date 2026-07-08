# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Shared Architecture-B harness for `Rails/IndexBy` and `Rails/IndexWith`.
      #
      # Both stock cops share the `RuboCop::Cop::IndexMethod` mixin, whose
      # autocorrect is heavy parser-AST geometry (strip prefix/suffix, rename
      # the method, rewrite block args, replace the body) plus cross-offense
      # `ignore_node` state. Rather than reproduce any of that, Rust nominates
      # candidate nodes (the four transform-to-hash shapes) once on the shared
      # walk; this module relocates each parser node (`Shirobai::NodeLocator`)
      # and runs stock's own `handle_possible_offense` / correction machinery
      # VERBATIM, so detection and `-A` bytes match stock by construction.
      #
      # Stock's node callbacks (`on_block` / `on_send` / `on_csend`, plus the
      # `on_numblock` / `on_itblock` aliases) are undefined so the Commissioner
      # never dispatches them per node; the same bodies are re-exposed as
      # `investigate_block` / `investigate_send` / `investigate_csend`, driven
      # only from `on_new_investigation`. The candidate list is a superset —
      # the empty-hash arg, block arity, key/value identity and Ruby-version
      # gate are all re-checked by the stock matchers the cop still owns.
      #
      # `ignore_node` is stock's real one (range/heredoc based, per cop
      # instance): candidates arrive in pre-order (an outer node before any it
      # contains), so a nested transform's inner offense is suppressed for
      # autocorrect exactly as stock does. IndexBy and IndexWith are separate
      # cop instances with separate `@ignored_nodes`, matching stock.
      module IndexMethodSupport
        def self.included(base)
          # Pull in stock's `Captures` / `Autocorrection` structs and the
          # private glue (`handle_possible_offense`, `extract_captures`,
          # `prepare_correction`, `execute_correction`), then drop the public
          # node callbacks so only `on_new_investigation` drives the cop.
          base.include RuboCop::Cop::IndexMethod
          base.include Shirobai::Cop::BundleEligible
          %i[on_block on_numblock on_itblock on_send on_csend].each do |m|
            base.send(:undef_method, m) if base.method_defined?(m)
          end
        end

        def on_new_investigation
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }
          located = Shirobai::NodeLocator.locate(processed_source, keys)

          keys.each do |key|
            node = located[key]
            next unless node

            case node.type
            when :block, :numblock, :itblock then investigate_block(node)
            when :send then investigate_send(node)
            when :csend then investigate_csend(node)
            end
          end
        end

        # --- stock `IndexMethod#on_block` / `on_send` / `on_csend`, copied
        # verbatim (rubocop-rails 2.35.5) and renamed so they are not node
        # callbacks. Every `on_bad_*` matcher and the private glue are the
        # included stock code.

        def investigate_block(node)
          on_bad_each_with_object(node) do |*match|
            handle_possible_offense(node, match, 'each_with_object')
          end

          return if target_ruby_version < 2.6

          on_bad_to_h(node) do |*match|
            handle_possible_offense(node, match, 'to_h { ... }')
          end
        end

        def investigate_send(node)
          on_bad_map_to_h(node) do |*match|
            handle_possible_offense(node, match, 'map { ... }.to_h')
          end

          on_bad_hash_brackets_map(node) do |*match|
            handle_possible_offense(node, match, 'Hash[map { ... }]')
          end
        end

        def investigate_csend(node)
          on_bad_map_to_h(node) do |*match|
            handle_possible_offense(node, match, 'map { ... }.to_h')
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte; a gated-off file (nil from Dispatch) falls back to the
        # standalone entry scanning `buffer.source`. The rails origin has no
        # per-file gate, so on a bundle-eligible file the bundle path always
        # wins.
        def resolved_candidates
          if bundle_eligible?
            ranges = Dispatch.offenses_for(processed_source, config, candidate_slot)
            return ranges unless ranges.nil?
          end
          Shirobai.check_rails_index_method(processed_source.buffer.source)
        end
      end
    end
  end
end
