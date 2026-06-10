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
          # not implemented yet
        end
      end
    end
  end
end
