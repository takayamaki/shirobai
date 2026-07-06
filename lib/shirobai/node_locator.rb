# frozen_string_literal: true

module Shirobai
  # Locates parser-gem AST nodes by their source range.
  #
  # Rust reports offsets on the prism AST; some Architecture-B cops need the
  # actual `Parser::AST::Node` at a reported range so they can run stock's
  # detection + autocorrect (which reason over the parser AST) VERBATIM. Given
  # a `ProcessedSource` and a list of `[begin_char, end_char]` ranges
  # (CHARACTER offsets — convert byte offsets with `SourceOffsets` first),
  # `locate` does ONE pre-order descent of the AST and returns a Hash mapping
  # each requested `[begin, end]` to the node whose expression range matches
  # it exactly.
  #
  # Pre-order means the SHALLOWEST node wins an exact-range tie (a wrapper node
  # sharing its single child's range resolves to the wrapper). The descent
  # stops as soon as every requested range has matched, so on the common file
  # the walk ends near the last target. Ranges with no matching node are simply
  # absent from the result.
  #
  # It does NOT prune subtrees by expression-range containment: a heredoc body
  # sits OUTSIDE the expression range of every ancestor between the heredoc
  # node and the root (their `loc.expression` stops at the `<<~` marker), so a
  # containment prune would drop candidates nested in heredoc interpolation
  # (`"#{user.errors.to_h}"`). `locate` runs only when the wrapper already has
  # candidates (a small fraction of files), and RuboCop walks the whole AST per
  # investigation anyway, so a full descent here is negligible and always
  # sound.
  #
  # (This is the plugin-neutral twin of `Shirobai::RSpec::NodeLocator`; the
  # rspec gem keeps its own copy so it needs no dependency on this file, and
  # the rails gem uses this one.)
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

    # Pre-order visit `node`; record it if its expression range is a target and
    # unseen, then recurse into every child. Returns `true` once all targets
    # are found so callers can unwind the descent immediately.
    def descend(node, targets, result)
      expr = node.loc&.expression
      if expr
        key = [expr.begin_pos, expr.end_pos]
        result[key] ||= node if targets.key?(key)
        return true if result.size == targets.size
      end

      node.children.each do |child|
        next unless child.is_a?(Parser::AST::Node)

        return true if descend(child, targets, result)
      end
      false
    end
    private_class_method :descend
  end
end
