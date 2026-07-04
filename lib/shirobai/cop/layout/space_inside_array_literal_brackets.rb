# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideArrayLiteralBrackets`.
      #
      # Rust walks the AST once, replicating stock's `on_array` /
      # `on_array_pattern` over square-bracket array literals and array
      # patterns (percent arrays and reference brackets never fire), including
      # stock's `find_node_with_brackets` redirect of array patterns to their
      # nearest constant pattern and its first/last bracket-token pair.
      #
      # The result is `[offenses, node_ops]`:
      #
      # - each offense is `[start, end, message_code, node, suppress]`;
      #   `suppress` mirrors stock's
      #   `autocorrect_with_disable_uncorrectable? && !start_ok` early return,
      #   so the offense is dropped when that mode is active;
      # - `node_ops[node]` is the node's corrector program
      #   (`SpaceCorrector` / `compact_corrections` reduced to
      #   remove / insert-after / insert-before calls), applied on the node's
      #   first offense like stock's `ignore_node` grouping.
      class SpaceInsideArrayLiteralBrackets < RuboCop::Cop::Base
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MESSAGES = [
          "Use space inside array brackets.",
          "Do not use space inside array brackets.",
          "Use one space inside empty array brackets.",
          "Do not use space inside empty array brackets."
        ].freeze

        STYLES = { "no_space" => 0, "space" => 1, "compact" => 2 }.freeze

        def self.cop_name = "Layout/SpaceInsideArrayLiteralBrackets"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[nums, lists]` with
        # `nums = [EnforcedStyle, EnforcedStyleForEmptyBrackets == 'space']`
        # and no lists.
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
          off = SourceOffsets.for(processed_source.raw_source)
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
            Dispatch.offenses_for(processed_source, config, :space_inside_array_literal_brackets)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_inside_array_literal_brackets(
              processed_source.raw_source, nums[0], nums[1] == 1
            )
          end
        end

        # No per-investigation state and no config that can't be packed, so
        # this cop is always bundle eligible. Kept for symmetry / a future
        # fallback.
        def bundle_eligible?
          true
        end
      end
    end
  end
end
