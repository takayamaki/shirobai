# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/SelfAssignment`.
      #
      # The detection (lvasgn/ivasgn/cvasgn/gvasgn/casgn rhs equals lhs name,
      # masgn pair-wise self-assignment, or_asgn/and_asgn, attribute setter and
      # `[]=` self-assignment) happens entirely in Rust. Ruby only:
      #
      # 1. Turns byte offsets into offenses.
      # 2. Filters by `AllowRBSInlineAnnotation` when that config is `true`
      #    — using stock's exact code path
      #    (`processed_source.ast_with_comments[node]`), keyed off the anchor
      #    node Rust returned per offense. When the config is the default
      #    `false` we skip the lookup entirely so the common case stays fast.
      class SelfAssignment < RuboCop::Cop::Base
        MSG = "Self-assignment detected."

        def self.cop_name = "Lint/SelfAssignment"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/SelfAssignment")

        # Config is fully handled on the Ruby side (RBS lookup needs
        # `processed_source.ast_with_comments`, only available here), so the
        # bundle takes no per-cop args.
        def self.bundle_args(_config) = []

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :self_assignment)
          return if offenses.empty?

          off = SourceOffsets.for(processed_source.raw_source)
          buffer = processed_source.buffer
          allow_rbs = !!cop_config["AllowRBSInlineAnnotation"]
          anchor_index = allow_rbs ? build_anchor_index : nil

          offenses.each do |start_offset, end_offset, anchor_offset|
            if allow_rbs && rbs_annotation_at?(anchor_index, anchor_offset)
              next
            end
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            add_offense(range)
          end
        end

        private

        # Builds a Hash mapping byte-end-offset -> AST node, restricted to the
        # node kinds stock passes into `rbs_inline_annotation?`. This is the
        # minimal AST scan we can do (one pass) so the Rust-returned anchor
        # offsets resolve to the same nodes stock would key with.
        def build_anchor_index
          ast = processed_source.ast
          return {} unless ast

          index = {}
          ast.each_node(*ANCHOR_NODE_TYPES) do |node|
            # Stock's targets:
            #   lvasgn/ivasgn/cvasgn/gvasgn -> node.rhs
            #   casgn                       -> node.rhs
            #   masgn                       -> first_lhs = node.lhs.assignments.first
            #   or_asgn / and_asgn          -> node.lhs
            #   send/csend (with []= or =)  -> node.receiver
            anchor = case node.type
                     when :lvasgn, :ivasgn, :cvasgn, :gvasgn, :casgn
                       node.rhs
                     when :masgn
                       node.lhs.assignments.first if node.lhs.respond_to?(:assignments)
                     when :or_asgn, :and_asgn
                       node.lhs
                     when :send, :csend
                       node.receiver
                     end
            next unless anchor.respond_to?(:loc)

            end_pos = anchor.loc.expression&.end_pos
            next unless end_pos

            # The Rust visitor only emits one offense per outer assignment, so
            # in practice each anchor offset maps to exactly one node. If two
            # candidate nodes end at the same byte, the last one wins — same
            # bytes mean stock's `awc[a]` and `awc[b]` would both consult the
            # same comment, so either lookup yields the same result.
            index[end_pos] = anchor
          end
          index
        end

        ANCHOR_NODE_TYPES = %i[lvasgn ivasgn cvasgn gvasgn casgn masgn or_asgn and_asgn send csend].freeze

        def rbs_annotation_at?(anchor_index, anchor_offset)
          # `processed_source.ast_with_comments[anchor].any? { |c| c.text.start_with?('#:') }`
          anchor = anchor_index[anchor_offset]
          return false unless anchor

          comments = processed_source.ast_with_comments[anchor]
          return false unless comments

          comments.any? { |comment| comment.text.start_with?("#:") }
        end
      end
    end
  end
end
