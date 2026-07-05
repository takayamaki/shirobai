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
    # The bundled (shared-walk) path scans `raw_source`; the standalone path
    # scans the parser-normalized `buffer.source`. The two agree only when they
    # are byte-identical (CRLF / BOM / `__END__` truncation break that), so a cop
    # may take the bundle path exactly then; otherwise it falls back so every
    # offset lines up with parser-gem's index.
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
