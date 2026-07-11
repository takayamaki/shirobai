# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust-accelerated reimplementation of `Style/MutableConstant`.
      #
      # `Style/MutableConstant` is almost entirely AST work: the `on_casgn`
      # value-mutability check (`literal_check` / `strict_check`), the
      # `shareable_constant_value` magic-comment scope scan (which reads
      # `processed_source.lines`, NOT the token stream), and the `.freeze`
      # autocorrect with all its branches (splat expansion, array bracketing,
      # range / send parenthesizing, recursive nested freezing). That part is
      # stock's, copied verbatim from
      # `vendor/rubocop/lib/rubocop/cop/style/mutable_constant.rb`, so detection
      # and `-A` bytes match stock exactly.
      #
      # The ONE place stock reaches the parser-gem token stream (the "toucher"
      # cost) is `FrozenStringLiteral#frozen_string_literals_enabled?`, via
      # `leading_comment_lines` -> `processed_source.tokens`. This wrapper
      # replaces ONLY that method with the token-free Rust leading-comment scan
      # (shared with `Lint/DuplicateMagicComment` /
      # `Style/FrozenStringLiteralComment` / `Style/EmptyLiteral`). Everything
      # else, including the whole `FrozenStringLiteral` mixin around it, stays
      # stock.
      class MutableConstant < RuboCop::Cop::Base
        # Stock's inline `ShareableConstantValue` module, copied verbatim except
        # for the fully-qualified `RuboCop::MagicComment` (stock resolves it
        # through its `RuboCop::Cop::Style` nesting; this file is nested under
        # `Shirobai::Cop::Style`). It reads `processed_source.lines`, not tokens,
        # so it is not a toucher and needs no Rust.
        module ShareableConstantValue
          module_function

          def recent_shareable_value?(node)
            shareable_constant_comment = magic_comment_in_scope node
            return false if shareable_constant_comment.nil?

            shareable_constant_value = RuboCop::MagicComment.parse(shareable_constant_comment)
                                                            .shareable_constant_value
            shareable_constant_value_enabled? shareable_constant_value
          end

          def magic_comment_in_scope(node)
            processed_source_till_node(node).reverse_each.find do |line|
              RuboCop::MagicComment.parse(line).valid_shareable_constant_value?
            end
          end

          private

          def processed_source_till_node(node)
            processed_source.lines[0..(node.last_line - 1)]
          end

          def shareable_constant_value_enabled?(value)
            %w[literal experimental_everything experimental_copy].include? value
          end
        end
        private_constant :ShareableConstantValue

        include ShareableConstantValue
        include RuboCop::Cop::FrozenStringLiteral
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        MSG = "Freeze mutable objects assigned to constants."

        def self.cop_name = "Style/MutableConstant"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Not a bundled cop (it dispatches `on_casgn` normally); the only Rust
        # call is the token-free `frozen_string_literals_enabled?` helper. Kept
        # for the 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_casgn(node)
          if node.expression.nil? # This is only the case for `CONST += ...` or similar
            parent = node.parent
            return unless parent.or_asgn_type? # We only care about `CONST ||= ...`

            on_assignment(parent.children.last)
          else
            on_assignment(node.expression)
          end
        end

        private

        def on_assignment(value)
          nodes = mutable_nodes(value) do |node|
            if style == :strict
              strict_check(node)
            else
              literal_check(node)
            end
          end

          nodes.each do |node|
            add_offense(node) { |corrector| autocorrect(corrector, node) }
          end
        end

        def mutable_nodes(value, &block)
          if recursive? && explicitly_frozen_literal?(value)
            literal_children(value.receiver).flat_map { |c| mutable_nodes(c, &block) }
          else
            node_offending = yield(value)

            if node_offending
              [value]
            else
              []
            end
          end
        end

        def strict_check(value)
          return if immutable_literal?(value)
          return if operation_produces_immutable_object?(value)
          return if frozen_string_literal?(value)
          return if shareable_constant_value?(value)

          true
        end

        def literal_check(value)
          return unless mutable_or_unfrozen_range?(value)
          return if frozen_string_literal?(value)
          return if shareable_constant_value?(value)

          true
        end

        def mutable_or_unfrozen_range?(value)
          mutable_literal?(value) ||
            (target_ruby_version <= 2.7 && range_enclosed_in_parentheses?(value))
        end

        def autocorrect(corrector, node)
          expr = node.source_range

          splat_value = splat_value(node)
          if splat_value
            correct_splat_expansion(corrector, expr, splat_value)
            corrector.insert_after(expr, ".freeze")
            return
          end

          if node.array_type? && !node.bracketed?
            corrector.wrap(expr, "[", "]")
          elsif requires_parentheses?(node)
            corrector.wrap(expr, "(", ")")
          end

          corrector.insert_after(expr, ".freeze")

          freeze_nested_literals(corrector, node) if recursive?
        end

        def freeze_nested_literals(corrector, node)
          literal_children(node).each do |child|
            if explicitly_frozen_literal?(child)
              freeze_nested_literals(corrector, child.receiver)
            elsif freezable_nested_literal?(child)
              autocorrect(corrector, child)
            end
          end
        end

        def freezable_nested_literal?(node)
          return false if frozen_string_literal?(node)
          return false if shareable_constant_value?(node)

          mutable_literal?(node)
        end

        def literal_children(node)
          case node.type
          when :array
            return [] if node.percent_literal?

            node.children
          when :hash
            node.children.flat_map { |child| child.pair_type? ? child.children : [] }
          else
            []
          end
        end

        def explicitly_frozen_literal?(node)
          return false unless node.send_type? && node.method?(:freeze)

          node.receiver && mutable_literal?(node.receiver)
        end

        def recursive?
          cop_config.fetch("Recursive", false)
        end

        def mutable_literal?(value)
          return false if frozen_regexp_or_range_literals?(value)

          value.mutable_literal?
        end

        def immutable_literal?(node)
          frozen_regexp_or_range_literals?(node) || node.immutable_literal?
        end

        def shareable_constant_value?(node)
          return false if target_ruby_version < 3.0

          recent_shareable_value? node
        end

        def frozen_regexp_or_range_literals?(node)
          target_ruby_version >= 3.0 && node.type?(:regexp, :range)
        end

        def requires_parentheses?(node)
          node.range_type? || (node.send_type? && node.loc.dot.nil?)
        end

        def correct_splat_expansion(corrector, expr, splat_value)
          if range_enclosed_in_parentheses?(splat_value)
            corrector.replace(expr, "#{splat_value.source}.to_a")
          else
            corrector.replace(expr, "(#{splat_value.source}).to_a")
          end
        end

        # @!method splat_value(node)
        def_node_matcher :splat_value, <<~PATTERN
          (array (splat $_))
        PATTERN

        # @!method operation_produces_immutable_object?(node)
        def_node_matcher :operation_produces_immutable_object?, <<~PATTERN
          {
            (const _ _)
            (send (const {nil? cbase} :Struct) :new ...)
            (block (send (const {nil? cbase} :Struct) :new ...) ...)
            (send _ :freeze)
            (send {float int} {:+ :- :* :** :/ :% :<<} _)
            (send _ {:+ :- :* :** :/ :%} {float int})
            (send _ {:== :=== :!= :<= :>= :< :>} _)
            (send (const {nil? cbase} :ENV) :[] _)
            (or (send (const {nil? cbase} :ENV) :[] _) _)
            (send _ {:count :length :size} ...)
            (block (send _ {:count :length :size} ...) ...)
          }
        PATTERN

        # @!method range_enclosed_in_parentheses?(node)
        def_node_matcher :range_enclosed_in_parentheses?, <<~PATTERN
          (begin (range _ _))
        PATTERN

        # Stock's `FrozenStringLiteral#frozen_string_literals_enabled?` with the
        # single token-touching leading-comment scan replaced by its token-free
        # Rust equivalent. The magic-comment resolution and the
        # `StringLiteralsFrozenByDefault` fallback are identical to stock:
        # `check_frozen_string_literals_enabled` returns the first leading magic
        # comment's `frozen_string_literal` value, or `sfbd_default` when none
        # (`nil`/`false` both map to `false`, exactly as stock's
        # `string_literals_frozen_by_default?.nil?` guard). Memoized per
        # `processed_source` (a reused cop instance investigates several sources).
        def frozen_string_literals_enabled?
          src = processed_source
          return false unless src.ruby_version

          unless defined?(@frozen_for) && @frozen_for.equal?(src)
            @frozen_for = src
            @frozen_enabled = Shirobai.check_frozen_string_literals_enabled(
              src.buffer.source, string_literals_frozen_by_default? == true
            )
          end
          @frozen_enabled
        end
      end
    end
  end
end
