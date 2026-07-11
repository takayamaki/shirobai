# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust-accelerated reimplementation of `Style/EmptyLiteral`.
      #
      # `Style/EmptyLiteral` is almost entirely AST work (const-receiver
      # `Array.new` / `Hash.new` / `String.new` / `Array[]` matchers plus the
      # parenless-argument-wrapping autocorrect), and that part is stock's, run
      # verbatim on the real parser AST — so detection and `-A` bytes match stock
      # exactly, including the `block` / `numblock` exclusion asymmetry between
      # Array and Hash. The ONE place it touches the parser-gem token stream (the
      # "toucher" cost) is the `String.new` branch's `frozen_strings?`, which
      # reaches `FrozenStringLiteral#leading_comment_lines` ->
      # `processed_source.tokens`. This wrapper replaces ONLY that method with a
      # token-free computation: Rust does the same leading-comment scan (shared
      # with `Lint/DuplicateMagicComment` / `Style/FrozenStringLiteralComment`)
      # from the prism parse, and the config half runs in Ruby. Everything else
      # is stock, copied verbatim from
      # `vendor/rubocop/lib/rubocop/cop/style/empty_literal.rb`.
      class EmptyLiteral < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        include RuboCop::Cop::StringLiteralsHelp
        extend RuboCop::Cop::AutoCorrector

        ARR_MSG = "Use array literal `[]` instead of `%<current>s`."
        HASH_MSG = "Use hash literal `{}` instead of `%<current>s`."
        STR_MSG = "Use string literal `%<prefer>s` instead of `String.new`."

        RESTRICT_ON_SEND = %i[new [] Array Hash].freeze

        def self.cop_name = "Style/EmptyLiteral"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Not a bundled cop (it dispatches on_send normally); the only Rust call
        # is the token-free `frozen_strings?` helper. Kept for the 4+1 convention.
        def self.bundle_args(_config)
          []
        end

        # @!method array_node(node)
        def_node_matcher :array_node, "(send (const {nil? cbase} :Array) :new (array)?)"

        # @!method hash_node(node)
        def_node_matcher :hash_node, "(send (const {nil? cbase} :Hash) :new)"

        # @!method str_node(node)
        def_node_matcher :str_node, "(send (const {nil? cbase} :String) :new)"

        # @!method array_with_block(node)
        def_node_matcher :array_with_block, "(block (send (const {nil? cbase} :Array) :new) args _)"

        # @!method hash_with_block(node)
        def_node_matcher :hash_with_block, <<~PATTERN
          {
            (block (send (const {nil? cbase} :Hash) :new) args _)
            (numblock (send (const {nil? cbase} :Hash) :new) ...)
          }
        PATTERN

        # @!method array_with_index(node)
        def_node_matcher :array_with_index, <<~PATTERN
          {
            (send (const {nil? cbase} :Array) :[])
            (send nil? :Array (array))
          }
        PATTERN

        # @!method hash_with_index(node)
        def_node_matcher :hash_with_index, <<~PATTERN
          {
            (send (const {nil? cbase} :Hash) :[])
            (send nil? :Hash (array))
          }
        PATTERN

        def on_send(node)
          return unless (message = offense_message(node))

          add_offense(node, message: message) do |corrector|
            corrector.replace(replacement_range(node), correction(node))
          end
        end

        private

        def offense_message(node)
          if offense_array_node?(node)
            format(ARR_MSG, current: node.source)
          elsif offense_hash_node?(node)
            format(HASH_MSG, current: node.source)
          elsif str_node(node) && !frozen_strings?
            format(STR_MSG, prefer: preferred_string_literal)
          end
        end

        def first_argument_unparenthesized?(node)
          parent = node.parent
          return false unless parent && %i[send super zsuper].include?(parent.type)

          node.equal?(parent.first_argument) && !parentheses?(node.parent)
        end

        def replacement_range(node)
          if hash_node(node) && first_argument_unparenthesized?(node)
            # `some_method {}` is not same as `some_method Hash.new`
            # because the braces are interpreted as a block. We will have
            # to rewrite the arguments to wrap them in parenthesis.
            args = node.parent.arguments

            range_between(args[0].source_range.begin_pos - 1, args[-1].source_range.end_pos)
          else
            node.source_range
          end
        end

        def offense_array_node?(node)
          (array_node(node) && !array_with_block(node.parent)) || array_with_index(node)
        end

        def offense_hash_node?(node)
          # If Hash.new takes a block, it can't be changed to {}.
          (hash_node(node) && !hash_with_block(node.parent)) || hash_with_index(node)
        end

        def correction(node)
          if offense_array_node?(node)
            "[]"
          elsif str_node(node)
            preferred_string_literal
          elsif offense_hash_node?(node)
            if first_argument_unparenthesized?(node)
              # `some_method {}` is not same as `some_method Hash.new`
              # because the braces are interpreted as a block. We will have
              # to rewrite the arguments to wrap them in parenthesis.
              args = node.parent.arguments
              "(#{args[1..].map(&:source).unshift('{}').join(', ')})"
            else
              "{}"
            end
          end
        end

        # Stock's `frozen_strings?`, with the two token-touching leading-comment
        # scans (`frozen_string_literals_enabled?` / `frozen_string_literals_disabled?`)
        # replaced by their token-free Rust equivalents. The config half is
        # identical to stock.
        def frozen_strings?
          return true if rust_frozen_string_literals_enabled?

          frozen_string_cop_enabled = config.cop_enabled?("Style/FrozenStringLiteralComment")
          frozen_string_cop_enabled &&
            !rust_frozen_string_literals_disabled? &&
            string_literals_frozen_by_default?.nil?
        end

        # Both scans are file-level; memoize per `processed_source` (a reused cop
        # instance investigates several sources in turn).
        def rust_frozen_string_literals_enabled?
          src = processed_source
          unless defined?(@frozen_for) && @frozen_for.equal?(src)
            @frozen_for = src
            source = src.buffer.source
            @frozen_enabled =
              Shirobai.check_frozen_string_literals_enabled(source, string_literals_frozen_by_default? == true)
            @frozen_disabled = Shirobai.check_frozen_string_literals_disabled(source)
          end
          @frozen_enabled
        end

        def rust_frozen_string_literals_disabled?
          rust_frozen_string_literals_enabled?
          @frozen_disabled
        end
      end
    end
  end
end
