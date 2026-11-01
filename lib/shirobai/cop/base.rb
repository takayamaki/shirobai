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
  end
end
