# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust-accelerated reimplementation of `Layout/RescueEnsureAlignment`.
      #
      # Stock's `on_new_investigation` materializes the parser-gem token stream
      # on EVERY file (the "toucher" cost) just to collect the keyword position
      # of each modifier `rescue` (`x rescue y`), which the cop uses to SKIP those
      # `resbody` nodes (a modifier rescue is not an alignment target). Everything
      # else — `on_resbody` / `on_ensure`, the `alignment_node` resolution, the
      # offense, and the autocorrect — is cheap AST work.
      #
      # This wrapper copies stock's body verbatim from
      # `vendor/rubocop/lib/rubocop/cop/layout/rescue_ensure_alignment.rb` and
      # replaces ONLY `on_new_investigation` (and its consumer `modifier?`): prism
      # separates a modifier rescue into its own node type (`RescueModifierNode`),
      # so the shared walk collects each one's keyword byte range with no token
      # stream. The wrapper turns those into the modifier-position set and runs
      # stock's detection / autocorrect unchanged, so offenses, messages, and
      # corrected bytes match stock by construction. The offense ranges come from
      # `node.loc.keyword` on the parser AST (never through Rust), so no
      # `SourceOffsets` conversion is needed for them; only the modifier keyword
      # positions (used purely for set membership) are byte-to-char converted.
      class RescueEnsureAlignment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::RangeHelp
        include RuboCop::Cop::EndKeywordAlignment
        extend RuboCop::Cop::AutoCorrector

        MSG = '`%<kw_loc>s` at %<kw_loc_line>d, %<kw_loc_column>d is not ' \
              'aligned with `%<beginning>s` at ' \
              '%<begin_loc_line>d, %<begin_loc_column>d.'
        ANCESTOR_TYPES = %i[kwbegin any_def class module sclass any_block].freeze
        ALTERNATIVE_ACCESS_MODIFIERS = %i[public_class_method private_class_method].freeze

        def self.cop_name = "Layout/RescueEnsureAlignment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed as a single enabled gate: 0 disabled (the shared walk skips the
        # modifier-rescue collection), 1 enabled.
        def self.bundle_args(config)
          [[config.for_badge(badge)["Enabled"] == false ? 0 : 1]]
        end

        def on_resbody(node)
          check(node) unless modifier?(node)
        end

        def on_ensure(node)
          check(node)
        end

        # Replaces stock's token-scan `on_new_investigation`: the modifier-rescue
        # keyword positions come from prism's `RescueModifierNode`s via Rust, not
        # `processed_source.tokens`.
        def on_new_investigation
          @modifier_positions = resolved_modifier_positions
        end

        private

        def resolved_modifier_positions
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          raw_positions(source).map { |begin_byte, end_byte| [off[begin_byte], off[end_byte]] }
        end

        def raw_positions(source)
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :rescue_ensure_alignment) || []
          else
            Shirobai.check_rescue_ensure_alignment(source)
          end
        end

        # Check alignment of node with rescue or ensure modifiers.

        def check(node)
          alignment_node = alignment_node(node)
          return if alignment_node.nil?

          alignment_loc = alignment_location(alignment_node)
          kw_loc        = node.loc.keyword

          return if alignment_loc.column == kw_loc.column || same_line?(alignment_loc, kw_loc)

          add_offense(
            kw_loc, message: format_message(alignment_node, alignment_loc, kw_loc)
          ) do |corrector|
            autocorrect(corrector, node, alignment_loc)
          end
        end

        def autocorrect(corrector, node, alignment_location)
          whitespace = whitespace_range(node)
          # Some inline node is sitting before current node.
          return nil unless whitespace.source.strip.empty?

          new_column = alignment_location.column

          corrector.replace(whitespace, ' ' * new_column)
        end

        def format_message(alignment_node, alignment_loc, kw_loc)
          format(
            MSG,
            kw_loc: kw_loc.source,
            kw_loc_line: kw_loc.line,
            kw_loc_column: kw_loc.column,
            beginning: alignment_source(alignment_node, alignment_loc),
            begin_loc_line: alignment_loc.line,
            begin_loc_column: alignment_loc.column
          )
        end

        # rubocop:disable Metrics/AbcSize, Metrics/MethodLength
        def alignment_source(node, starting_loc)
          ending_loc =
            case node.type
            when :block, :numblock, :itblock, :kwbegin
              node.loc.begin
            when :def, :defs, :class, :module,
                 :lvasgn, :ivasgn, :cvasgn, :gvasgn, :casgn
              node.loc.name
            when :sclass
              node.identifier.source_range
            when :masgn
              node.lhs.source_range
            else
              # It is a wrapper with receiver of object attribute or access modifier.
              node.receiver&.source_range || node.child_nodes.first.loc.name
            end

          range_between(starting_loc.begin_pos, ending_loc.end_pos).source
        end
        # rubocop:enable Metrics/AbcSize, Metrics/MethodLength

        # We will use ancestor or wrapper with access modifier.

        def alignment_node(node)
          ancestor_node = ancestor_node(node)

          return ancestor_node if ancestor_node.nil? || ancestor_node.kwbegin_type?
          return if ancestor_node.respond_to?(:send_node) &&
                    aligned_with_line_break_method?(ancestor_node, node)

          assignment_node = assignment_node(ancestor_node)
          return assignment_node if same_line?(ancestor_node, assignment_node)

          access_modifier_node = access_modifier_node(ancestor_node)
          return access_modifier_node unless access_modifier_node.nil?

          ancestor_node
        end

        def ancestor_node(node)
          node.each_ancestor(*ANCESTOR_TYPES).first
        end

        def aligned_with_line_break_method?(ancestor_node, node)
          send_node_loc = ancestor_node.send_node.loc
          do_keyword_line = ancestor_node.loc.begin.line
          rescue_keyword_column = node.loc.keyword.column
          selector = send_node_loc.respond_to?(:selector) ? send_node_loc.selector : send_node_loc

          if aligned_with_leading_dot?(do_keyword_line, send_node_loc, rescue_keyword_column)
            return true
          end

          do_keyword_line == selector&.line && rescue_keyword_column == selector.column
        end

        def aligned_with_leading_dot?(do_keyword_line, send_node_loc, rescue_keyword_column)
          return false unless send_node_loc.respond_to?(:dot) && (dot = send_node_loc.dot)

          do_keyword_line == dot.line && rescue_keyword_column == dot.column
        end

        def assignment_node(node)
          assignment_node = node.ancestors.first
          return nil unless
            assignment_node&.assignment?

          assignment_node
        end

        def access_modifier_node(node)
          return nil unless node.any_def_type?

          access_modifier_node = node.ancestors.first
          return nil unless access_modifier?(access_modifier_node)

          access_modifier_node
        end

        # Replaces stock's `@modifier_locations.include?(node.loc.keyword)`
        # (Parser::Source::Range membership) with an equivalent begin/end-offset
        # set membership. The positions come from prism's `RescueModifierNode`s,
        # exactly the tokens stock's `token.rescue_modifier?` scan would collect.
        def modifier?(node)
          return false if @modifier_positions.nil?

          kw = node.loc.keyword
          @modifier_positions.include?([kw.begin_pos, kw.end_pos])
        end

        def whitespace_range(node)
          begin_pos      = node.loc.keyword.begin_pos
          current_column = node.loc.keyword.column

          range_between(begin_pos - current_column, begin_pos)
        end

        def access_modifier?(node)
          return true if node.respond_to?(:access_modifier?) && node.access_modifier?

          return true if node.respond_to?(:method_name) &&
                         ALTERNATIVE_ACCESS_MODIFIERS.include?(node.method_name)

          false
        end

        def alignment_location(alignment_node)
          if begin_end_alignment_style == 'start_of_line'
            start_line_range(alignment_node)
          elsif alignment_node.any_block_type?
            # If the alignment node is a block, the `rescue`/`ensure` keyword should
            # be aligned to the start of the block. It is possible that the block's
            # `send_node` spans multiple lines, in which case it should align to the
            # start of the last line.
            send_node = alignment_node.send_node
            range = processed_source.buffer.line_range(send_node.last_line)
            range.adjust(begin_pos: range.source =~ /\S/)
          else
            alignment_node.source_range
          end
        end

        def begin_end_alignment_style
          begin_end_alignment_conf = config.for_cop('Layout/BeginEndAlignment')

          begin_end_alignment_conf['Enabled'] && begin_end_alignment_conf['EnforcedStyleAlignWith']
        end
      end
    end
  end
end
