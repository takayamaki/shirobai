# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/ArgumentAlignment`.
      #
      # Rust parses the source, walks every multi-argument method call, picks the
      # alignment base for the configured `EnforcedStyle`
      # (`with_first_argument` / `with_fixed_indentation`) and returns each
      # misaligned argument as an offense range plus its `column_delta`. Ruby
      # supplies the flattened config and applies the realignment via
      # `AlignmentCorrector` (the same division of labour as the multiline
      # indentation cops). Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the config derivation is purely config-driven,
      # so this cop is always bundle-eligible.
      class ArgumentAlignment < RuboCop::Cop::Base
        include RuboCop::Cop::Alignment
        extend RuboCop::Cop::AutoCorrector

        ALIGN_PARAMS_MSG = "Align the arguments of a method call if they span more than one line."

        FIXED_INDENT_MSG = "Use one level of indentation for arguments " \
                           "following the first line of a multi-line method call."

        def self.cop_name = "Layout/ArgumentAlignment"
        def self.badge = RuboCop::Cop::Badge.parse("Layout/ArgumentAlignment")

        # Packed args for the bundled run: `[style, indentation_width,
        # incompatible]`. `incompatible` replicates the instance derivation
        # exactly: it is only true for the explicit `with_first_argument` style
        # combined with a separator-aligned `Layout/HashAlignment`.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          enforced_style = cop_config["EnforcedStyle"]
          incompatible = enforced_style == "with_first_argument" &&
                         RuboCop::Cop::Layout::HashAlignment::SEPARATOR_ALIGNMENT_STYLES.any? do |sep_style|
                           config.for_enabled_cop("Layout/HashAlignment")[sep_style]&.include?("separator")
                         end
          [
            enforced_style == "with_fixed_indentation" ? 1 : 0,
            cop_config["IndentationWidth"] || config.for_cop("Layout/IndentationWidth")["Width"] || 2,
            incompatible
          ]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          message = fixed_indentation? ? FIXED_INDENT_MSG : ALIGN_PARAMS_MSG

          offenses = Dispatch.offenses_for(processed_source, config, :argument_alignment)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start, fin, column_delta, autocorrect|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            # Split on the per-offense correctability flag rather than testing it
            # inside the corrector block. Note this must stay keyed on the flag,
            # NOT on `autocorrect?` mode: RuboCop yields the block even in lint
            # mode and a non-empty corrector is what marks the offense
            # `:uncorrected` (correctable) to match stock. Skipping the block for
            # non-correctable offenses only avoids an unused Corrector allocation.
            unless autocorrect
              add_offense(range, message: message)
              next
            end

            add_offense(range, message: message) do |corrector|
              RuboCop::Cop::AlignmentCorrector.correct(corrector, processed_source, range, column_delta)
            end
          end
        end

        private

        # Config-derived and stable for the life of the instance; shares the
        # derivation with the bundled run (single source of truth).
        def bundle_args
          @bundle_args ||= self.class.bundle_args(config)
        end

        def fixed_indentation?
          bundle_args[0] == 1
        end
      end
    end
  end
end
