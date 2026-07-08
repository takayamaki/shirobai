# frozen_string_literal: true

require_relative "index_method_support"

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust-backed reimplementation of `Rails/IndexWith`
      # (rubocop-rails 2.35.5), Architecture B.
      #
      # Same harness as `IndexBy` (shared candidate list, `IndexMethodSupport`
      # relocation + stock detection/autocorrect). The `def_node_matcher`
      # patterns below are stock's IndexWith matchers (value-transform: key ==
      # element). The `minimum_target_rails_version 6.0` gate is stock's, driven
      # by the same `requires_gem` / `TargetRailsVersion` machinery the runner
      # already consults, so the wrapper is enabled on exactly the same targets
      # as stock.
      class IndexWith < RuboCop::Cop::Base
        include Shirobai::Cop::Rails::IndexMethodSupport
        extend RuboCop::Cop::AutoCorrector
        extend RuboCop::Cop::TargetRailsVersion

        minimum_target_rails_version 6.0

        def self.cop_name = "Rails/IndexWith"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.bundle_args(_config)
          []
        end

        def_node_matcher :on_bad_each_with_object, <<~PATTERN
          (block
            (call _ :each_with_object (hash))
            (args (arg $_el) (arg _memo))
            (call (lvar _memo) :[]= (lvar _el) $!`_memo))
        PATTERN

        def_node_matcher :on_bad_to_h, <<~PATTERN
          {
            (block
              (call _ :to_h)
              (args (arg $_el))
              (array (lvar _el) $_))
            (numblock
              (call _ :to_h) $1
              (array (lvar :_1) $_))
            (itblock
              (call _ :to_h) $:it
              (array (lvar :it) $_))
          }
        PATTERN

        def_node_matcher :on_bad_map_to_h, <<~PATTERN
          (call
            {
              (block
                (call _ {:map :collect})
                (args (arg $_el))
                (array (lvar _el) $_))
              (numblock
                (call _ {:map :collect}) $1
                (array (lvar :_1) $_))
              (itblock
                (call _ {:map :collect}) $:it
                (array (lvar :it) $_))
            }
            :to_h)
        PATTERN

        def_node_matcher :on_bad_hash_brackets_map, <<~PATTERN
          (send
            (const {nil? cbase} :Hash)
            :[]
            {
              (block
                (call _ {:map :collect})
                (args (arg $_el))
                (array (lvar _el) $_))
              (numblock
                (call _ {:map :collect}) $1
                (array (lvar :_1) $_))
              (itblock
                (call _ {:map :collect}) $:it
                (array (lvar :it) $_))
            }
          )
        PATTERN

        private

        def candidate_slot = :rails_index_with

        def new_method_name
          'index_with'
        end
      end
    end
  end
end
