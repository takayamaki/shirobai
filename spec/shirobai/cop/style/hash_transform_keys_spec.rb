# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::HashTransformKeys, :config do
  # `:ruby24, unsupported_on: :prism` and `:ruby25, unsupported_on: :prism`
  # contexts assert "when TargetRubyVersion < 2.5/2.6 stock does NOT flag".
  # shirobai parses with prism's Latest version and ignores TargetRuby (see
  # handoff §7-4: "TargetRubyVersion 対応は先延ばし"), so we cannot honour
  # `minimum_target_ruby_version`-driven skips. Stage these two cases out.
  VendorSpecHelper.load_vendor_spec(
    self,
    "rubocop/cop/style/hash_transform_keys_spec.rb",
    pending: [
      /below Ruby 2\.5 does not flag/,
      /below Ruby 2\.6 does not flag/
    ]
  )
end
