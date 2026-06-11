# frozen_string_literal: true

module Shirobai
  module Cop
    module Lint
      # Drop-in Rust reimplementation of `Lint/Debugger`.
      #
      # All detection (Prism parse, AST walk, offense location) happens in Rust.
      # Ruby only flattens the configuration and turns the byte offsets Rust
      # returns into offenses.
      class Debugger < RuboCop::Cop::Base
        MSG = 'Remove debugger entry point `%<source>s`.'

        def self.cop_name = "Lint/Debugger"
        def self.badge = RuboCop::Cop::Badge.parse("Lint/Debugger")

        def on_new_investigation
          source = processed_source.raw_source
          Shirobai.check_debugger(source, debugger_methods, debugger_requires).each do |start_offset, end_offset|
            range = Parser::Source::Range.new(processed_source.buffer, start_offset, end_offset)
            add_offense(range, message: format(MSG, source: range.source))
          end
        end

        private

        # The flattened lists are config-derived and an instance's config never
        # changes, so build them once instead of per file.
        def debugger_methods
          @debugger_methods ||= flatten_config("DebuggerMethods")
        end

        def debugger_requires
          @debugger_requires ||= flatten_config("DebuggerRequires")
        end

        def flatten_config(key)
          config = cop_config.fetch(key, [])
          list = config.is_a?(Array) ? config : config.values.flatten
          list.grep(String)
        end
      end
    end
  end
end
