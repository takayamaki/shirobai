# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/Void`.
      #
      # Rust parses the source and replicates stock's statement-position
      # analysis: parser `begin`-equivalent sequences (multi-statement bodies,
      # parentheses, keyword `begin`), single-expression block bodies
      # (`on_block`) and ensure branches (`on_ensure`), with the
      # `in_void_context?` parents (`initialize`/setter defs, `each`/`tap`
      # blocks, `for` bodies, `ensure` branches) and the each-block operator
      # suppression. Every offense category (operator / variable / constant /
      # literal / `self` / `defined?` / lambda-or-proc / nonmutating method)
      # carries its stock message and its correction as a Rust-computed
      # replace + remove pair; the no-correction cases (conditional branch
      # bodies, assignment-method defs) come back with both ranges empty so the
      # corrector block still runs and stays empty, exactly like stock's
      # bail-out paths. Ruby supplies the `CheckForMethodsWithNoSideEffects`
      # flag and applies the ranges. Offenses come from the per-file bundled
      # run (`Shirobai::Dispatch`); the config is one boolean, so this cop is
      # always bundle-eligible.
      class Void < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        # Referenced by the vendor spec (`described_class::BINARY_OPERATORS`).
        BINARY_OPERATORS = %i[* / % + - == === != < > <= >= <=>].freeze

        def self.cop_name = "Lint/Void"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/Void")

        # Packed args for the bundled run: `[check_for_methods_with_no_side_effects]`.
        def self.bundle_args(config)
          [!!config.for_badge(badge)["CheckForMethodsWithNoSideEffects"]]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :void)
          offenses.each do |start, fin, message, rep_start, rep_end, replacement, rem_start, rem_end|
            range = Parser::Source::Range.new(buffer, start, fin)
            # Stock yields the corrector block for every offense (the
            # uncorrectable cases simply leave it empty).
            add_offense(range, message: message) do |corrector|
              if rep_end > rep_start
                corrector.replace(Parser::Source::Range.new(buffer, rep_start, rep_end), replacement)
              end
              if rem_end > rem_start
                corrector.remove(Parser::Source::Range.new(buffer, rem_start, rem_end))
              end
            end
          end
        end
      end
    end
  end
end
