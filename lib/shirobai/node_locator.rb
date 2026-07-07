# frozen_string_literal: true

module Shirobai
  # Locates parser-gem AST nodes by their source range.
  #
  # Rust reports offsets on the prism AST; some Architecture-B cops need the
  # actual `Parser::AST::Node` at a reported range so they can run stock's
  # detection + autocorrect (which reason over the parser AST) VERBATIM. Given
  # a `ProcessedSource` and a list of `[begin_char, end_char]` ranges
  # (CHARACTER offsets — convert byte offsets with `SourceOffsets` first),
  # `locate` returns a Hash mapping each requested `[begin, end]` to the node
  # whose expression range matches it exactly.
  #
  # The SHALLOWEST node wins an exact-range tie (a wrapper node sharing its
  # single child's range resolves to the wrapper). Ranges with no matching
  # node are simply absent from the result. The result is identical to a
  # plain full pre-order descent — the two phases below are only an
  # optimization and never change which node a range resolves to.
  #
  # Two phases, because two forces pull against each other:
  #
  # 1. Pruned descent (fast, the common case). Pre-order, but skip any child
  #    subtree whose expression range contains no remaining target: a target
  #    lives inside its ancestors' ranges, so such a subtree cannot hold one.
  #    The walk cost is then proportional to the paths down to the targets,
  #    not the whole AST. Record exact-range matches shallowest-wins and
  #    early-exit once every target is found. A wrapper sharing its child's
  #    range still has that range CONTAIN the target range, so the prune never
  #    skips it — shallowest-wins survives pruning.
  #
  # 2. Full-descent fallback (slow, rare). A heredoc body sits OUTSIDE the
  #    expression range of every ancestor between the heredoc node and the
  #    root (their `loc.expression` stops at the `<<~` marker), so a target
  #    nested in heredoc interpolation (`"#{user.errors.to_h}"`) is pruned
  #    away in phase 1. ONLY when targets remain unfound do we pay for a full
  #    pre-order descent (no pruning) to locate the stragglers, again with the
  #    early-exit. Prune alone is unsound (drops heredoc-interior targets);
  #    full descent alone is slow on large spec files (walks the whole AST
  #    even when targets sit late); the fallback pays the full walk only when
  #    a target actually hides in a heredoc.
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

      # Phase 1: pruned descent handles every non-heredoc target quickly.
      descend_pruned(ast, targets, result)

      # Phase 2: only if a target escaped phase 1 (heredoc interior), pay for a
      # full descent to find whatever is still missing.
      descend_full(ast, targets, result) if result.size < targets.size

      result
    end

    # Pre-order visit `node`; record it if its expression range is a target and
    # unseen, then recurse only into children whose range could still contain a
    # remaining target. Returns `true` once all targets are found so callers can
    # unwind the descent immediately.
    def descend_pruned(node, targets, result)
      expr = node.loc&.expression
      if expr
        key = [expr.begin_pos, expr.end_pos]
        result[key] ||= node if targets.key?(key)
        return true if result.size == targets.size
      end

      node.children.each do |child|
        next unless child.is_a?(Parser::AST::Node)

        child_expr = child.loc&.expression
        # Prune: a child with a known range that contains no target cannot
        # lead to one (targets are contained in their ancestors' ranges).
        next if child_expr && !contains_any?(child_expr, targets)

        return true if descend_pruned(child, targets, result)
      end
      false
    end
    private_class_method :descend_pruned

    # Pre-order visit `node` with NO pruning; record it if its expression range
    # is a target and unseen, then recurse into every child. Returns `true` once
    # all targets are found so callers can unwind the descent immediately. This
    # is the sound-but-slow path used only for targets phase 1 could not reach.
    def descend_full(node, targets, result)
      expr = node.loc&.expression
      if expr
        key = [expr.begin_pos, expr.end_pos]
        result[key] ||= node if targets.key?(key)
        return true if result.size == targets.size
      end

      node.children.each do |child|
        next unless child.is_a?(Parser::AST::Node)

        return true if descend_full(child, targets, result)
      end
      false
    end
    private_class_method :descend_full

    # Does `[range.begin_pos, range.end_pos]` contain any target range?
    def contains_any?(range, targets)
      b = range.begin_pos
      e = range.end_pos
      targets.each_key.any? { |(tb, te)| b <= tb && te <= e }
    end
    private_class_method :contains_any?
  end
end
