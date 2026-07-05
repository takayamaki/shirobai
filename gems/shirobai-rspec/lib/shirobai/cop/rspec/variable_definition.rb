# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/VariableDefinition`
      # (rubocop-rspec 3.10.2).
      #
      # Rust classifies once on the shared walk: the candidate matcher and the
      # top-level-spec-group gate are exactly `RSpec/VariableName`'s (a
      # send-shaped `let`/`subject` with a literal first argument inside a
      # top-level spec group). The `EnforcedStyle` decides the offenders:
      # `symbols` (default) flags plain `str` names, `strings` flags `sym`
      # AND `dsym` names; a `dstr` name is never flagged (probed). Rust already
      # applies the style, so the wire carries only real offenders.
      #
      # The wrapper reproduces stock's `correct_variable`: a sym becomes
      # `value.inspect` (`variable.value.to_s.inspect`), a str becomes
      # `value.to_sym.inspect`, and a dsym becomes its source minus the leading
      # colon (`variable.source[1..]`).
      class VariableDefinition < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector
        include RuboCop::Cop::ConfigurableEnforcedStyle

        MSG = "Use %<style>s for variable names."

        def self.cop_name = "RSpec/VariableDefinition"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]` (0 symbols, 1 strings).
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("EnforcedStyle", "symbols") == "strings" ? 1 : 0]
        end

        def on_new_investigation
          offenses = Dispatch.offenses_for(processed_source, config, :rspec_variable_definition)
          # Gated-off file (see Dispatch#offenses_for): standalone fallback.
          offenses ||= Shirobai.check_rspec_variable_definition(
            processed_source.raw_source, *Shirobai::RSpec.segment(config)
          )
          return if offenses.empty?

          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |(start, fin, kind, value)|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: format(MSG, style: style)) do |corrector|
              corrector.replace(range, correct_variable(range, kind, value))
            end
          end
        end

        private

        # Reproduces stock `correct_variable`, keyed by the Rust `kind`
        # (0 sym / 1 str / 2 dsym). Only one kind reaches this per style, so
        # the mapping is unambiguous.
        def correct_variable(range, kind, value)
          case kind
          when 2 then range.source[1..]  # dsym: source minus the leading colon
          when 0 then value.inspect      # sym  (strings): value.to_s.inspect
          else value.to_sym.inspect      # str  (symbols): value.to_sym.inspect
          end
        end
      end
    end
  end
end
