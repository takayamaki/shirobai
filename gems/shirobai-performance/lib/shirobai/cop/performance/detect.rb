# frozen_string_literal: true

module Shirobai
  module Cop
    module Performance
      # Drop-in Rust reimplementation of `Performance/Detect`
      # (rubocop-performance 1.26.1).
      #
      # Rust replicates the four stock pattern branches (`select` /
      # `find_all` / `filter` chained with `first` / `last` / `[0]` /
      # `[-1]`, block and block-pass forms) including the
      # `accept_first_call?` gates (empty block body, non-block-pass args,
      # `lazy` chains) and the parser-semantics offense range (inner
      # selector through outer selector, where the sugar index form's
      # selector is the whole `[0]` bracket construct). The wrapper applies
      # the Rust-computed removal/replacement ranges — including stock's
      # knowingly broken rewrite of the explicit `.[](0)` form, byte for
      # byte. Messages come from Rust, formatted with the preferred method
      # resolved from `Style/CollectionMethods` below.
      class Detect < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Performance/Detect"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[preferred_method]`. Stock reads
        # `config.for_cop('Style/CollectionMethods')['PreferredMethods']['detect']`
        # and falls back to `detect`; the extra `|| {}` only guards configs
        # where stock would crash (bare `RuboCop::Config.new` without the
        # default `PreferredMethods` hash — packing runs for every config,
        # not just the ones that instantiate this cop).
        def self.bundle_args(config)
          preferred =
            (config.for_cop("Style/CollectionMethods")["PreferredMethods"] || {})["detect"] ||
            "detect"
          [preferred.to_s]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :perf_detect)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |sel_start, sel_end, recv_end, outer_end, message, replacement|
            range = Parser::Source::Range.new(buffer, off[sel_start], off[outer_end])
            add_offense(range, message: message) do |corrector|
              corrector.remove(Parser::Source::Range.new(buffer, off[recv_end], off[outer_end]))
              corrector.replace(
                Parser::Source::Range.new(buffer, off[sel_start], off[sel_end]), replacement
              )
            end
          end
        end
      end
    end
  end
end
