# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideReferenceBrackets`.
      #
      # Rust checks the bracket pair of every index reference (`CallNode`
      # `[]` / `[]=` plus prism's `Index*Write` / `IndexTarget` assignment
      # forms, which legacy parser folds into `send :[]` / `send :[]=`),
      # reproducing stock's empty-brackets handling (before the multiline
      # guard), the legacy node extent for `multiline?`, and the `[ \t]`
      # space tests.
      #
      # The result is `[offenses, node_ops]` (same shape as
      # `SpaceInsideArrayLiteralBrackets`):
      #
      # - each offense is `[start, end, message_code, node, suppress]`;
      #   `suppress` mirrors stock's
      #   `autocorrect_with_disable_uncorrectable? && !start_ok` early return
      #   on the right-bracket offense;
      # - `node_ops[node]` is the node's corrector program (`SpaceCorrector`
      #   reduced to remove / insert-after / insert-before calls), applied on
      #   the node's first offense like stock's `ignore_node` grouping.
      class SpaceInsideReferenceBrackets < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Use space inside reference brackets.",
          "Do not use space inside reference brackets.",
          "Use one space inside empty reference brackets.",
          "Do not use space inside empty reference brackets."
        ].freeze

        STYLES = { "no_space" => 0, "space" => 1 }.freeze

        def self.cop_name = "Layout/SpaceInsideReferenceBrackets"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [EnforcedStyle, EnforcedStyleForEmptyBrackets == 'space']`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [
            [
              STYLES.fetch(cop_config["EnforcedStyle"] || "no_space"),
              cop_config["EnforcedStyleForEmptyBrackets"] == "space" ? 1 : 0
            ],
            []
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          offenses, node_ops = result_for_source
          drop_suppressed = autocorrect_with_disable_uncorrectable?
          corrected = {}

          offenses.each do |start, fin, code, node, suppress|
            next if suppress && drop_suppressed

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MESSAGES.fetch(code)) do |corrector|
              unless corrected[node]
                apply_ops(corrector, buffer, off, node_ops.fetch(node))
              end
              corrected[node] = true
            end
          end
        end

        private

        # Replays the node's corrector program with the same corrector calls
        # stock makes (`remove` / `insert_after` / `insert_before`).
        def apply_ops(corrector, buffer, off, ops)
          ops.each do |op, start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            case op
            when 0 then corrector.remove(range)
            when 1 then corrector.insert_after(range, " ")
            else corrector.insert_before(range, " ")
            end
          end
        end

        def result_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_inside_reference_brackets)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_inside_reference_brackets(
              processed_source.buffer.source, nums[0], nums[1] == 1
            )
          end
        end

        # See `SpaceInsideParens#bundle_eligible?`.
        def bundle_eligible?
          return @bundle_eligible unless @bundle_eligible.nil?

          @bundle_eligible = processed_source.buffer.source == processed_source.raw_source
        end
      end
    end
  end
end
