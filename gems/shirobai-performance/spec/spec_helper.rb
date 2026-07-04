# frozen_string_literal: true

# Spec helper for the shirobai-performance gem. Reuses the core suite's
# helper (vendor spec loader, EdgeCaseParity, RSpec config) and adds:
#
# - the plugin gem itself (which requires rubocop-performance first and
#   replaces the stock Performance cops — see lib/shirobai-performance.rb)
# - the vendor spec root for the rubocop-performance submodule
#
# rubocop-performance's config/default.yml is merged into
# `RuboCop::ConfigLoader.default_configuration` automatically:
# `rubocop/rspec/support`'s CopHelper `before(:all)` integrates every
# activated gem with `default_lint_roller_plugin` metadata, and this
# suite's bundle (gems/shirobai-performance/Gemfile) activates
# rubocop-performance.
require_relative "../../../spec/spec_helper"
require "shirobai-performance"

module PerformanceVendorSpecHelper
  VENDOR_SPEC_ROOT = File.expand_path("../../../vendor/rubocop-performance/spec", __dir__)

  def self.load_vendor_spec(example_group, relative_path, pending: [])
    VendorSpecHelper.load_vendor_spec(
      example_group, relative_path, pending: pending, root: VENDOR_SPEC_ROOT
    )
  end
end
