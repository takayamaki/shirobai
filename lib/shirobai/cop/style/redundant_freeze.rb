# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/RedundantFreeze`.
      #
      # Detection and autocorrect both happen in Rust; Ruby turns each tuple
      # into an offense on the send node and two `corrector.remove` calls (the
      # `.` and the `freeze` selector), matching stock's
      # `corrector.remove(node.loc.dot)` + `corrector.remove(node.loc.selector)`.
      #
      # The Rust side reproduces the `FrozenStringLiteral` mixin's
      # `frozen_string_literals_enabled?` on raw bytes, so this cop no longer
      # needs the parser token stream the mixin's `leading_comment_lines` would
      # otherwise build.
      #
      # `bundle_args` carries the two config-derived booleans the cop depends on
      # (`AllCops/TargetRubyVersion >= 3.0` and whether
      # `AllCops/StringLiteralsFrozenByDefault` is literally `true`); everything
      # else is decided from the source, so the bundle path is always taken.
      class RedundantFreeze < RuboCop::Cop::Base
        extend RuboCop::Cop::AutoCorrector

        MSG = "Do not freeze immutable objects, as freezing them has no effect."

        def self.cop_name = "Style/RedundantFreeze"
        def self.badge = RuboCop::Cop::Badge.parse("Style/RedundantFreeze")

        # Packed config nums: `[target_ruby_30_plus, string_literals_frozen_by_default]`.
        # `string_literals_frozen_by_default?` may be `nil`/`false`/`true`; only
        # a literal `true` enables the fallback (nil and false both mean "not
        # frozen by default"), so we pass a plain boolean.
        def self.bundle_args(config)
          [[
            config.target_ruby_version >= 3.0 ? 1 : 0,
            config.string_literals_frozen_by_default? == true ? 1 : 0
          ]]
        end

        def bundle_eligible?
          true
        end

        def on_new_investigation
          buffer = processed_source.buffer
          off = SourceOffsets.for(processed_source.raw_source)

          fetch_offenses.each do |off_start, off_end, dot_start, dot_end, sel_start, sel_end|
            range = Parser::Source::Range.new(buffer, off[off_start], off[off_end])
            add_offense(range) do |corrector|
              dot = Parser::Source::Range.new(buffer, off[dot_start], off[dot_end])
              selector = Parser::Source::Range.new(buffer, off[sel_start], off[sel_end])
              corrector.remove(dot)
              corrector.remove(selector)
            end
          end
        end

        private

        def fetch_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :redundant_freeze)
          else
            nums = self.class.bundle_args(config).first
            Shirobai.check_redundant_freeze(
              processed_source.raw_source, nums[0] == 1, nums[1] == 1
            )
          end
        end
      end
    end
  end
end
