# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/FirstArgumentIndentation`.
      #
      # Rust parses the source, walks every method call with arguments, decides
      # the alignment base for the configured `EnforcedStyle`, and returns the
      # first argument as an offense range plus its `column_delta`, the message,
      # the `within?` autocorrect flag and the range to realign (which for
      # `special_for_inner_method_call_in_parentheses` may be the whole receiver
      # chain). Ruby supplies the flattened config and applies the realignment
      # via `AlignmentCorrector` (the same division of labour as the other
      # alignment cops). Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the config derivation is purely config-driven,
      # so this cop is always bundle-eligible.
      class FirstArgumentIndentation < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        include RuboCop::Cop::ConfigurableEnforcedStyle
        extend RuboCop::Cop::AutoCorrector

        STYLES = {
          "special_for_inner_method_call_in_parentheses" => 0,
          "consistent" => 1,
          "consistent_relative_to_receiver" => 2,
          "special_for_inner_method_call" => 3
        }.freeze

        def self.cop_name = "Layout/FirstArgumentIndentation"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/FirstArgumentIndentation")

        # Packed args for the bundled run: `[style, indentation_width,
        # enforce_fixed_with_no_line_break]`. The enforce flag replicates the
        # instance derivation: `Layout/ArgumentAlignment` enforcing
        # `with_fixed_indentation` while `Layout/FirstMethodArgumentLineBreak`
        # is disabled.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          argument_alignment_config = config.for_enabled_cop("Layout/ArgumentAlignment")
          enforce = argument_alignment_config["EnforcedStyle"] == "with_fixed_indentation" &&
                    !config.cop_enabled?("Layout/FirstMethodArgumentLineBreak")
          [
            STYLES.fetch(cop_config["EnforcedStyle"], 0),
            cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2,
            enforce
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :first_argument_indentation)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, column_delta, message, autocorrect, cs, ce|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # Always pass a block so the offense is correctable, matching stock:
            # the Alignment mixin's `register_offense` always hands `add_offense`
            # a block, even for the `within?` case (`autocorrect: false`). A
            # blockless `add_offense` would mark the offense uncorrectable. When
            # `autocorrect` is false the block returns early, leaving an empty
            # (no-op) corrector, so the offense stays correctable but is not
            # rewritten this pass (see argument_alignment).
            add_offense(range, message: message) do |corrector|
              next unless autocorrect

              correct_range = Parser::Source::Range.new(buffer, off[cs], off[ce])
              RuboCop::Cop::AlignmentCorrector.correct(
                corrector, processed_source, correct_range, column_delta
              )
            end
          end
        end
      end
    end
  end
end
