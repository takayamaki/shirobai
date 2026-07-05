# frozen_string_literal: true

module Shirobai
  module Cop
    module Style
      # Drop-in Rust reimplementation of `Style/FileNull`.
      #
      # Rust reproduces the whole detection: the file-level `/dev/null` tally
      # that gates a bare `nul` (stock's `@contain_dev_null_string_in_file`),
      # the `acceptable?` array / hash-pair exemption, the `valid_string?`
      # empty / invalid-encoding guard, and the `REGEXP` full match against
      # `/dev/null` / `NUL` / `NUL:` (case-insensitive). It returns one
      # `[start, end, message]` per offense (message carries the original-case
      # source value); the offense range is also the `File::NULL` replace
      # range, so the wrapper just applies them. Config-less, so it is always
      # bundle-eligible for config purposes; the shared `bundle_eligible?`
      # guard only guards the raw-vs-buffer byte identity (CRLF / BOM).
      class FileNull < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible
        extend RuboCop::Cop::AutoCorrector

        def self.cop_name = "Style/FileNull"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less: contributes nothing to nums / lists. Kept for the
        # 4+1 single-source-of-config convention.
        def self.bundle_args(_config)
          []
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start, fin, message|
            range = Parser::Source::Range.new(buffer, off[start], off[fin])
            add_offense(range, message: message) do |corrector|
              corrector.replace(range, "File::NULL")
            end
          end
        end

        private

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :file_null)
          else
            Shirobai.check_file_null(processed_source.buffer.source)
          end
        end
      end
    end
  end
end
