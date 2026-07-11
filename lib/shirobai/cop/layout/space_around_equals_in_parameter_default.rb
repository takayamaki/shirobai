# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceAroundEqualsInParameterDefault`.
      #
      # Rust walks the AST once and emits one `[start, end]` range per offending
      # optarg — `range_between(arg.end_pos, value.begin_pos)`, stock's offense
      # range — computed straight from the `OptionalParameterNode`'s `name_loc`
      # / `operator_loc` / `value` positions and the two `space_after?` byte
      # checks (`\s` at the arg-name end and at the `=` end). This replaces
      # stock's `processed_source.tokens_within(node)` token access, the "toucher"
      # cost, with prism-native node positions.
      #
      # The wrapper reproduces stock's message (`missing` under `space`,
      # `detected` under `no_space`) and its `/=\s*(\S+)/` autocorrect verbatim
      # on the Ruby side, so detection and autocorrect are byte-identical.
      class SpaceAroundEqualsInParameterDefault < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        include RuboCop::Cop::ConfigurableEnforcedStyle
        include RuboCop::Cop::RangeHelp
        extend RuboCop::Cop::AutoCorrector

        MSG = "Surrounding space %<type>s in default value assignment."

        STYLE_TO_U8 = {
          space: 0,
          no_space: 1
        }.freeze

        def self.cop_name = "Layout/SpaceAroundEqualsInParameterDefault"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[style]`.
        def self.bundle_args(config)
          own = config.for_badge(badge)
          style = STYLE_TO_U8.fetch((own["EnforcedStyle"] || "space").to_sym, 0)
          [style]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          type = style == :space ? "missing" : "detected"

          offenses_for_source.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: format(MSG, type: type)) do |corrector|
              autocorrect(corrector, range)
            end
          end
        end

        private

        # Stock's `autocorrect`, verbatim.
        def autocorrect(corrector, range)
          m = range.source.match(/=\s*(\S+)/)
          rest = m ? m.captures[0] : ""
          replacement = style == :space ? " = " : "="
          corrector.replace(range, replacement + rest)
        end

        def offenses_for_source
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_around_equals_in_parameter_default)
          else
            Shirobai.check_space_around_equals_in_parameter_default(
              processed_source.buffer.source,
              STYLE_TO_U8.fetch(style, 0)
            )
          end
        end
      end
    end
  end
end
