# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/FirstHashElementIndentation`.
      #
      # Rust parses the source, walks every braced hash literal (replicating the
      # `each_argument_node` / `ignore_node` claiming of hashes by a method
      # call's left parenthesis), decides the alignment base for the configured
      # `EnforcedStyle` (brace / after-paren / parent hash key / start of line),
      # applies the `Layout/HashAlignment` separator longest-key offset when
      # configured, and returns each misindented first pair and hanging right
      # brace as an offense range plus its `column_delta`, message and an
      # autocorrect-target marker. Ruby supplies the flattened config and
      # applies the realignment via `AlignmentCorrector` (the same division of
      # labour as `FirstArrayElementIndentation`). Offenses come from the
      # per-file bundled run (`Shirobai::Dispatch`); the config derivation is
      # purely config-driven, so this cop is always bundle-eligible.
      class FirstHashElementIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "special_inside_parentheses" => 0,
          "consistent" => 1,
          "align_braces" => 2
        }.freeze

        def self.cop_name = "Layout/FirstHashElementIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/FirstHashElementIndentation")

        # Packed args for the bundled run: `[style, indentation_width,
        # enforce_fixed, colon_separator, rocket_separator]`. The enforce flag
        # replicates `enforce_first_argument_with_fixed_indentation?`: when
        # `Layout/ArgumentAlignment` enforces `with_fixed_indentation`, the cop
        # stops letting a `(` claim a hash (it does NOT stand the cop down, and
        # there is no style exemption — unlike the array cop). The separator
        # flags replicate `separator_style?`, which reads
        # `Layout/HashAlignment`'s `Enforced{Colon,HashRocket}Style`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          arg_alignment_config = config.for_enabled_cop("Layout/ArgumentAlignment")
          hash_alignment_config = config.for_cop("Layout/HashAlignment")
          [
            STYLES.fetch(cop_config["EnforcedStyle"], 0),
            cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2,
            arg_alignment_config["EnforcedStyle"] == "with_fixed_indentation",
            hash_alignment_config["EnforcedColonStyle"] == "separator",
            hash_alignment_config["EnforcedHashRocketStyle"] == "separator"
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :first_hash_element_indentation)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, column_delta, message, correct_target|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              target = correction_target(range, correct_target, buffer)
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, target, column_delta
              )
            end
          end
        end

        private

        # Mirrors stock's `autocorrect`:
        #   - right brace (`correct_target == -1`): realign the brace's RANGE;
        #   - first pair whose value begins after its key (`0`): realign only
        #     the key's line (`buffer.line_range(node.loc.line)`);
        #   - first pair whose value begins on the same line as (or before) its
        #     key (`1`): realign the whole pair NODE (so `AlignmentCorrector`
        #     skips lines inside its string literals / heredocs).
        def correction_target(range, correct_target, buffer)
          case correct_target
          when 1
            node_for(range) || range
          when 0
            buffer.line_range(range.line)
          else
            range
          end
        end

        # The outermost AST node whose source range is exactly `range`
        # (pre-order, so the pair wins over any same-range descendant), or nil.
        def node_for(range)
          root = processed_source.ast
          return nil unless root

          root.each_node.find do |node|
            sr = node.source_range
            sr && sr.begin_pos == range.begin_pos && sr.end_pos == range.end_pos
          end
        end
      end
    end
  end
end
