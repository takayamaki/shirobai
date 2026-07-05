# frozen_string_literal: true

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/VariableName`
      # (rubocop-rspec 3.10.2).
      #
      # Rust classifies once on the shared walk: a candidate is a
      # send-shaped `let`/`subject` (any block kind, or no block at all)
      # with a plain sym/str first argument, inside a TOP-LEVEL spec group
      # (stock's `inside_example_group?` checks the outermost enclosing
      # statement, not just any ancestor — a group wrapped in a top-level
      # class does not count). Rust also evaluates stock's
      # `ConfigurableNaming::FORMATS` regexps (probed: `[[:lower:]]` /
      # `[[:upper:]]` are the Unicode Lowercase/Uppercase properties,
      # `\d` is ASCII), and returns the failing candidates plus the
      # passing (value, kind) pairs.
      #
      # The wrapper replays stock's reporting exactly: AllowedPatterns are
      # Ruby regexps so they are applied here, passing values drive
      # `correct_style_detected`, and each offense's block reports the
      # opposing style for `--auto-gen-config` (the alternative style when
      # the name is valid there, otherwise "unrecognized"). Stock's
      # `class_emitter_method?` escape hatch is dead code on send nodes,
      # so `valid_name?` reduces to the regexp.
      class VariableName < RuboCop::Cop::Base
        include RuboCop::Cop::ConfigurableNaming
        include RuboCop::Cop::AllowedPattern

        MSG = "Use %<style>s for variable names."

        def self.cop_name = "RSpec/VariableName"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]` (0 snake_case,
        # 1 camelCase).
        def self.bundle_args(config)
          [config.for_badge(badge).fetch("EnforcedStyle", "snake_case") == "camelCase" ? 1 : 0]
        end

        def on_new_investigation
          data = Dispatch.offenses_for(processed_source, config, :rspec_variable_name)
          # Gated-off file (see Dispatch#offenses_for): standalone fallback.
          data ||= Shirobai.check_rspec_variable_name(
            processed_source.raw_source, *Shirobai::RSpec.segment(config)
          )
          offenses, passing = data
          return if offenses.empty? && passing.empty?

          passing.each do |(value, kind)|
            correct_style_detected unless matches_allowed_pattern?(wrap(value, kind))
          end
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)
          offenses.each do |(start, fin, kind, value, valid_alt)|
            next if matches_allowed_pattern?(wrap(value, kind))

            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: format(MSG, style: style)) do
              if valid_alt
                unexpected_style_detected(alternative_style)
              else
                unrecognized_style_detected
              end
            end
          end
        end

        private

        # Stock passes `variable.value` around: a Symbol for sym nodes, a
        # String for str nodes (both respond to the pattern predicates).
        def wrap(value, kind)
          kind.zero? ? value.to_sym : value
        end
      end
    end
  end
end
