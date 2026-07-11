# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceInsideStringInterpolation`.
      #
      # Stock is an `Interpolation` + `SurroundingSpace` cop: for every
      # interpolation `#{...}` it reads `processed_source.tokens_within(begin_node)`
      # to find the `#{` / `}` delimiters and the space just inside them — a
      # per-interpolation token-stream access (the "toucher" cost). Rust does the
      # whole check from the delimiter byte positions and emits, per offending
      # delimiter, the offense range plus the autocorrect edits; the wrapper only
      # builds ranges and replays the edits.
      #
      # The autocorrect (stock's `SpaceCorrector`) runs once per interpolation
      # (stock's `ignore_node`), so Rust attaches all of an interpolation's edits
      # to its FIRST offense; the wrapper applies them only on that offense.
      class SpaceInsideStringInterpolation < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "%<command>s space inside string interpolation."

        STYLE_TO_U8 = {
          no_space: 0,
          space: 1
        }.freeze

        COMMANDS = {
          0 => "Do not use",
          1 => "Use"
        }.freeze

        def self.cop_name = "Layout/SpaceInsideStringInterpolation"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`.
        def self.bundle_args(config)
          own = config.for_badge(badge)
          [STYLE_TO_U8.fetch((own["EnforcedStyle"] || "no_space").to_sym, 0)]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(bundle_eligible? ? processed_source.raw_source : buffer.source)

          offenses_for_source.each do |start, fin, command, edits|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: format(MSG, command: COMMANDS.fetch(command))) do |corrector|
              edits.each do |edit_start, edit_end, insert|
                if insert
                  corrector.insert_before(
                    Parser::Source::Range.new(buffer, off[edit_start], off[edit_start]), " "
                  )
                else
                  corrector.remove(Parser::Source::Range.new(buffer, off[edit_start], off[edit_end]))
                end
              end
            end
          end
        end

        private

        def offenses_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_inside_string_interpolation)
          else
            Shirobai.check_space_inside_string_interpolation(
              processed_source.buffer.source,
              STYLE_TO_U8.fetch(style, 0)
            )
          end
        end
      end
    end
  end
end
