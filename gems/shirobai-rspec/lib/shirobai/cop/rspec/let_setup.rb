# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/LetSetup`
      # (rubocop-rspec 3.10.2).
      #
      # Everything is computed on the shared walk: query roots are
      # scope-change frames (example/shared groups with an rspec receiver
      # and `include_*` blocks), candidates are collected `let`s literally
      # named `let!` with one plain sym/str name, an inner `let!` shadowed
      # check compares (kind, value) — `let!('w')` and `let!(:w)` do not
      # shadow each other — and "used" means a receiverless ZERO-argument
      # send with the same name anywhere in the root's subtree (stock's
      # `(send nil? %)` search pattern has no argument wildcard, so `w(1)`
      # and `w(&b)` are not uses while `w { }` is). Probed quirks live as
      # differential specs in let_setup_edge_cases_spec.rb.
      class LetSetup < RuboCop::Cop::Base
        MSG = "Do not use `let!` to setup objects not referenced in tests."

        def self.cop_name = "RSpec/LetSetup"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less (the segment's role lists cover everything).
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :rspec_let_setup)
          # Gated-off file (see Dispatch#offenses_for): standalone fallback.
          offenses ||= Shirobai.check_rspec_let_setup(
            processed_source.raw_source, *Shirobai::RSpec.segment(config)
          )
          return if offenses.empty?

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |(start, fin)|
            add_offense(Parser::Source::Range.new(buffer, off[start], off[fin]))
          end
        end
      end
    end
  end
end
