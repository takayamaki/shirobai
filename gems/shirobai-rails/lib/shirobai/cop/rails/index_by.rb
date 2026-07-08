# frozen_string_literal: true

require_relative "index_method_support"

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust-backed reimplementation of `Rails/IndexBy`
      # (rubocop-rails 2.35.5), Architecture B.
      #
      # Rust nominates the four transform-to-hash candidate shapes on the shared
      # walk; `IndexMethodSupport` relocates each parser node and runs stock's
      # `IndexMethod` detection + autocorrect VERBATIM. The `def_node_matcher`
      # patterns and `new_method_name` below are copied unchanged from stock, so
      # the key-transform (value == element) matching, the `ignore_node`
      # cross-offense state and the byte-exact autocorrect are all stock's own.
      class IndexBy < RuboCop::Cop::Base
        include Shirobai::Cop::Rails::IndexMethodSupport
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Rails/IndexBy"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # No behavioral config on the Rust side (candidates only); unused by the
        # rails segment, kept for parity with the sibling wrappers.
        def self.bundle_args(_config)
          []
        end

        def_node_matcher :on_bad_each_with_object, <<~PATTERN
          (block
            (call _ :each_with_object (hash))
            (args (arg $_el) (arg _memo))
            (call (lvar _memo) :[]= $!`_memo (lvar _el)))
        PATTERN

        def_node_matcher :on_bad_to_h, <<~PATTERN
          {
            (block
              (call _ :to_h)
              (args (arg $_el))
              (array $_ (lvar _el)))
            (numblock
              (call _ :to_h) $1
              (array $_ (lvar :_1)))
            (itblock
              (call _ :to_h) $:it
              (array $_ (lvar :it)))
          }
        PATTERN

        def_node_matcher :on_bad_map_to_h, <<~PATTERN
          (call
            {
              (block
                (call _ {:map :collect})
                (args (arg $_el))
                (array $_ (lvar _el)))
              (numblock
                (call _ {:map :collect}) $1
                (array $_ (lvar :_1)))
              (itblock
                (call _ {:map :collect}) $:it
                (array $_ (lvar :it)))
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
                (array $_ (lvar _el)))
              (numblock
                (call _ {:map :collect}) $1
                (array $_ (lvar :_1)))
              (itblock
                (call _ {:map :collect}) $:it
                (array $_ (lvar :it)))
            }
          )
        PATTERN

        private

        def candidate_slot = :rails_index_by

        def new_method_name
          'index_by'
        end
      end
    end
  end
end
