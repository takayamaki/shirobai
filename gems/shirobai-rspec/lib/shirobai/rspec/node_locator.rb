# frozen_string_literal: true

module Shirobai
  module RSpec
    # Locates parser-gem AST nodes by their source range.
    #
    # Rust reports offsets on the prism AST; a few RSpec cops need the actual
    # `Parser::AST::Node` at some of those offsets (e.g. to run stock's
    # structural `#uniq` / `#==` over them, which byte comparison cannot
    # reproduce). Given a `ProcessedSource` and a list of `[begin_char,
    # end_char]` ranges (CHARACTER offsets — convert byte offsets with
    # `SourceOffsets` first), `locate` does ONE pre-order descent of the AST
    # and returns a Hash mapping each requested `[begin, end]` to the node
    # whose expression range matches it exactly.
    #
    # Pre-order means the SHALLOWEST node wins an exact-range tie (a wrapper
    # node sharing its single child's range resolves to the wrapper). Subtrees
    # whose range cannot contain any remaining target are pruned, so the walk
    # cost is proportional to the paths down to the targets, not the whole AST.
    # Ranges with no matching node are simply absent from the result.
    module NodeLocator
      module_function

      # @param processed_source [RuboCop::ProcessedSource]
      # @param ranges [Array<Array(Integer, Integer)>] `[begin_char, end_char]`
      # @return [Hash{Array(Integer, Integer) => Parser::AST::Node}]
      def locate(processed_source, ranges)
        result = {}
        ast = processed_source.ast
        return result if ast.nil? || ranges.empty?

        targets = {}
        ranges.each { |range| targets[range] = true }
        descend(ast, targets, result)
        result
      end

      # Pre-order visit `node`; record it if its expression range is a target
      # and unseen, then recurse into children that could still contain a
      # target.
      def descend(node, targets, result)
        expr = node.loc&.expression
        if expr
          key = [expr.begin_pos, expr.end_pos]
          result[key] ||= node if targets.key?(key)
        end

        node.children.each do |child|
          next unless child.is_a?(Parser::AST::Node)

          child_expr = child.loc&.expression
          # Prune: a child with a known range that contains no target cannot
          # lead to one (targets are contained in their ancestors' ranges).
          next if child_expr && !contains_any?(child_expr, targets)

          descend(child, targets, result)
        end
      end
      private_class_method :descend

      # Does `[range.begin_pos, range.end_pos]` contain any target range?
      def contains_any?(range, targets)
        b = range.begin_pos
        e = range.end_pos
        targets.each_key.any? { |(tb, te)| b <= tb && te <= e }
      end
      private_class_method :contains_any?
    end
  end
end
