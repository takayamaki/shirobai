# frozen_string_literal: true

require "rubocop"

module Shirobai
  module Cop
    # Class-level replica of `RuboCop::Cop::AllowedMethods#allowed_methods`,
    # for `bundle_args` class methods that must derive a cop's config without
    # a cop instance (`cop_config` is `config.for_badge(badge)`).
    def self.allowed_methods_config(cop_config)
      deprecated = Array(cop_config.fetch("IgnoredMethods", [])) +
                   Array(cop_config.fetch("ExcludedMethods", []))
      allowed = Array(cop_config.fetch("AllowedMethods", []))
      allowed += deprecated unless deprecated.any?(Regexp)
      allowed
    end

    # Shared `bundle_eligible?` for wrapper cops.
    #
    # A cop may take the bundle path only when `buffer.source == raw_source`;
    # otherwise (CRLF, a non-UTF-8 magic encoding) it calls its standalone
    # entry point on `buffer.source`. This guard dates from when the bundled
    # path scanned `raw_source`. Both paths now scan `buffer.source`
    # (see `Dispatch`), so the fallback only costs speed on such files; the
    # offsets line up either way.
    #
    # The verdict is memoized, but the memo is guarded on the `processed_source`
    # identity. RuboCop's real CLI builds a fresh cop per file, yet a reused
    # instance (vendor specs, and any future RuboCop change to instance reuse)
    # investigates several sources in turn. A plain `@bundle_eligible` /
    # `.nil?` / `defined?` memo would freeze the first file's verdict and leak it
    # onto later files with a different eligibility; keying on the source
    # identity recomputes it on each new investigation.
    module BundleEligible
      private

      def bundle_eligible?
        src = processed_source
        return @bundle_eligible if defined?(@bundle_eligible_for) && @bundle_eligible_for.equal?(src)

        @bundle_eligible_for = src
        @bundle_eligible = src.buffer.source == src.raw_source
      end
    end
  end
end
