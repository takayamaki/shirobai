# frozen_string_literal: true

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/SpaceBeforeComment`.
      #
      # Stock pairs `sorted_tokens` and flags a comment token that starts
      # exactly where the previous same-line token ends. Byte-side that is
      # "the byte before the comment exists and is not whitespace": tokens
      # are separated by whitespace only, a comment never shares its line
      # with a later token, and `=begin` docs sit at column 0. Comments come
      # from the shared prism parse (one comment per `=begin`/`=end` block,
      # matching parser-gem's single `tCOMMENT`).
      #
      # Each offense is the `[start, end)` range of the comment; the
      # corrector inserts a space before it.
      class SpaceBeforeComment < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        MSG = "Put a space before an end-of-line comment."

        def self.cop_name = "Layout/SpaceBeforeComment"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to `nums` / `lists`.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: MSG) do |corrector|
              corrector.insert_before(range, " ")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :space_before_comment)
          else
            Shirobai.check_space_before_comment(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
