# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/HashEachMethods`.
      #
      # Rust parses the source and replicates every stock pattern branch:
      # `hash.keys.each` / `hash.values.each` blocks and `&:sym` block-pass
      # forms (rewritten to `each_key` / `each_value`), and two-argument
      # `each` blocks with an unused key/value argument (selector rename plus
      # unused-argument removal), including the `handleable?` gates
      # (array-converter receivers, hash mutation inside the block, literal
      # receivers) and the `AllowedReceivers` receiver-name matching. Ruby
      # supplies the `AllowedReceivers` list and applies the Rust-computed
      # replacement/removal ranges. Offenses come from the per-file bundled
      # run (`Shirobai::Dispatch`); the list is plain strings, so this cop is
      # always bundle-eligible.
      class HashEachMethods < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Style/HashEachMethods"
        def self.badge = RuboCop::Cop::Badge.parse("Style/HashEachMethods")

        # Packed args for the bundled run: `[allowed_receivers]`, replicating
        # `AllowedReceivers#allowed_receivers` stringified for Rust.
        def self.bundle_args(config)
          [Array(config.for_badge(badge)["AllowedReceivers"]).map(&:to_s)]
        end

        def on_new_investigation
          buffer = processed_source.buffer

          offenses = Dispatch.offenses_for(processed_source, config, :hash_each_methods)
          offenses.each do |start, fin, message, rep_start, rep_end, replacement, rem_start, rem_end|
            range = Parser::Source::Range.new(buffer, start, fin)
            # Stock yields the corrector block for every offense.
            add_offense(range, message: message) do |corrector|
              corrector.replace(Parser::Source::Range.new(buffer, rep_start, rep_end), replacement)
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
