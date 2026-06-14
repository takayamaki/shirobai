# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/PercentLiteralDelimiters`.
      #
      # Rust walks every percent literal once and decides on a per-type
      # `PreferredDelimiters` lookup (resolved Ruby-side from `default` +
      # per-type overrides) whether the literal:
      #
      # - already uses the preferred opener, OR
      # - contains the preferred opener/closer in any str/sym child source
      #   (stock's `contains_preferred_delimiter?`), OR
      # - (for `%w` / `%i` only) contains the begin delimiter's matchpair
      #   character — `(`/`)` / `[`/`]` / `{`/`}` / `<`/`>` or the mirror
      #   single-byte forms — in a child source (stock's
      #   `include_same_character_as_used_for_delimiter?`).
      #
      # When none of the skips apply, Rust returns the offense range plus the
      # `opening_loc` and the single-byte closer (so regex options like the
      # trailing `i` in `%r(.*)i` survive the autocorrect, matching stock's
      # `node.loc.end` byte width). The wrapper builds the offense + corrector,
      # formatting both with the resolved preferred pair for the literal's type.
      class PercentLiteralDelimiters < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "`%<type>s`-literals should be delimited by `%<open>s` and `%<close>s`."

        # Canonical type order shared with `crates/.../percent_literal_delimiters.rs`
        # and stock's `PreferredDelimiters::PERCENT_LITERAL_TYPES`.
        TYPES = ["%", "%i", "%I", "%q", "%Q", "%r", "%s", "%w", "%W", "%x"].freeze

        def self.cop_name = "Style/PercentLiteralDelimiters"
        def self.badge = RuboCop::Cop::Badge.parse("Style/PercentLiteralDelimiters")

        # Returns `[pairs]` — a 10-entry list of two-byte strings packed for
        # the bundle path's `lists` slot. We resolve `default` + per-type
        # overrides exactly like stock's `PreferredDelimiters#preferred_delimiters`:
        # when `default` is set, every type defaults to it (then overridden by
        # its own key); without `default` the listed types still resolve.
        def self.bundle_args(config)
          cfg = config.for_badge(badge)
          configured = cfg.fetch("PreferredDelimiters", {})
          # Stock raises ArgumentError on an unknown key. We mirror that here
          # so the `from_packed` side never sees a missing-type fallback. The
          # raise propagates through `Dispatch.bundle_token`, which surfaces
          # it just like stock would.
          invalid = configured.keys - (TYPES + ["default"])
          unless invalid.empty?
            raise ArgumentError, "Invalid preferred delimiter config key: #{invalid.join(', ')}"
          end

          default = configured["default"]
          pairs = TYPES.map do |type|
            configured[type] || default
          end
          if pairs.any?(&:nil?)
            # Missing without `default` => the type has no preferred pair; we
            # use `()` as an inert placeholder. The cop never reaches the
            # `process` arm for this type (stock would also bail out), but
            # `from_packed` insists on 10 entries.
            pairs = pairs.map { |p| p || "()" }
          end
          [pairs]
        end

        def bundle_eligible?
          true
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          resolved_result.each do |start_offset, end_offset, begin_start, begin_end,
                                   end_start, end_end, type_index|
            type = TYPES[type_index]
            open_delim, close_delim = preferred_pair_for(type).chars

            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            message = format(MSG, type: type, open: open_delim, close: close_delim)
            add_offense(range, message: message) do |corrector|
              begin_range = Parser::Source::Range.new(
                buffer, off[begin_start], off[begin_end]
              )
              end_range = Parser::Source::Range.new(
                buffer, off[end_start], off[end_end]
              )
              corrector.replace(begin_range, "#{type}#{open_delim}")
              corrector.replace(end_range, close_delim)
            end
          end
        end

        private

        def resolved_result
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :percent_literal_delimiters)
          else
            pairs = self.class.bundle_args(config).first
            Shirobai.check_percent_literal_delimiters(processed_source.raw_source, pairs)
          end
        end

        def preferred_pair_for(type)
          self.class.bundle_args(config).first[TYPES.index(type)]
        end
      end
    end
  end
end
