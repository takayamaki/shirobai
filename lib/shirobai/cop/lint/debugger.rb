# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/Debugger`.
      #
      # All detection (Prism parse, AST walk, offense location) happens in Rust.
      # Ruby only flattens the configuration and turns the byte offsets Rust
      # returns into offenses. Offenses come from the per-file bundled run
      # (`Shirobai::Dispatch`); the config derivation is purely config-driven,
      # so this cop is always bundle-eligible.
      class Debugger < RuboCop::Cop::Base
        MSG = 'Remove debugger entry point `%<source>s`.'

        def self.cop_name = "Lint/Debugger"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/Debugger")

        # Packed args for the bundled run: `[debugger_methods, debugger_requires]`,
        # the exact values `Shirobai.check_debugger` receives.
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [flatten_config(cop_config, "DebuggerMethods"), flatten_config(cop_config, "DebuggerRequires")]
        end

        def self.flatten_config(cop_config, key)
          value = cop_config.fetch(key, [])
          list = value.is_a?(Array) ? value : value.values.flatten
          list.grep(String)
        end
        private_class_method :flatten_config

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :debugger)
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |start_offset, end_offset|
            range = Parser::Source::Range.new(processed_source.buffer, off[start_offset], off[end_offset])
            add_offense(range, message: format(MSG, source: range.source))
          end
        end
      end
    end
  end
end
