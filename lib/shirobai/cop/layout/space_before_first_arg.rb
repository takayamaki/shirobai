# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeFirstArg`.
      #
      # Rust replays stock's `on_send` / `on_csend` over prism `CallNode`s
      # (parenless, non-operator, non-setter calls with arguments — a
      # block-pass counts like in parser-gem) and the whole
      # `PrecedingFollowingAlignment` scan for `AllowForAlignment`: nearest
      # candidate line per direction (comments and blanks transparent), the
      # indentation-filtered second pass, char-column `aligned_words?`, and
      # the assignment-token alignment for `:sym=`-shaped arguments (found
      # with a masked longest-match operator scan).
      #
      # Each offense is the `[start, end)` whitespace run before the first
      # argument (possibly empty for glued arguments); the corrector replaces
      # it with a single space.
      class SpaceBeforeFirstArg < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Put one space between the method name and the first argument."

        def self.cop_name = "Layout/SpaceBeforeFirstArg"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        def self.autocorrect_incompatible_with
          [RuboCop::Cop::Style::MethodCallWithArgsParentheses]
        end

        # Packed args for the bundled run: `[[AllowForAlignment], []]`
        # (`nil` counts as falsy, like stock's `allow_for_alignment?`).
        def self.bundle_args(config)
          cop_config = config.for_badge(badge)
          [[cop_config["AllowForAlignment"] ? 1 : 0], []]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.replace(range, " ")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_before_first_arg)
          else
            nums, = self.class.bundle_args(config)
            Shirobai.check_space_before_first_arg(processed_source.buffer.source, nums[0] == 1)
          end
        end
      end
    end
  end
end
