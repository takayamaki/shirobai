# frozen_string_literal: true

# Spec helper for the shirobai-rspec gem. Reuses the core suite's helper
# (vendor spec loader, EdgeCaseParity, RSpec config) and adds:
#
# - the plugin gem itself (which requires rubocop-rspec first and replaces
#   the stock RSpec cops — see lib/shirobai-rspec.rb)
# - the vendor spec root for the rubocop-rspec submodule
# - rubocop-rspec's own spec support: the `_spec.rb` default filename for
#   ExpectOffense (RSpec cops are departmental and would not run on the
#   plain default filename), the `with default RSpec/Language config`
#   shared context, and the smoke-test shared examples (spec/smoke_tests
#   is a symlink into the submodule because the upstream glob is relative
#   to the working directory).
#
# rubocop-rspec's config/default.yml is merged into
# `RuboCop::ConfigLoader.default_configuration` automatically:
# `rubocop/rspec/support`'s CopHelper `before(:all)` integrates every
# activated gem with `default_lint_roller_plugin` metadata, and this
# suite's bundle (gems/shirobai-rspec/Gemfile) activates rubocop-rspec.
require_relative "../../../spec/spec_helper"
require "shirobai-rspec"

# Upstream spec support, pulled from the pinned submodule so vendor specs
# run against exactly the shared contexts they were written for.
require_relative "../../../vendor/rubocop-rspec/spec/support/expect_offense"
require_relative "../../../vendor/rubocop-rspec/spec/shared/smoke_test_examples"
require_relative "../../../vendor/rubocop-rspec/spec/shared/detects_style_behavior"
# The gem-shipped shared context that seeds `other_cops` with the default
# RSpec/Language config (resolves inside the rubocop-rspec gem, not
# rubocop's own `rubocop/rspec/shared_contexts`).
require "rubocop/rspec/shared_contexts/default_rspec_language_config_context"

RSpec.configure do |config|
  # Mirror upstream's derived metadata for OUR cop spec paths: every file
  # under spec/shirobai/cop/rspec/ is a cop spec with `:config`, gets the
  # default Language shared context, and runs the upstream smoke tests.
  config.define_derived_metadata(
    file_path: %r{/spec/shirobai/cop/rspec/}
  ) do |meta|
    meta[:type] = :cop_spec
  end
  config.define_derived_metadata(type: :cop_spec) do |meta|
    meta[:config] = true
  end

  config.include(ExpectOffense)
  config.include_context "with default RSpec/Language config", :config
  config.include_context "smoke test", type: :cop_spec
end

module RSpecVendorSpecHelper
  VENDOR_SPEC_ROOT = File.expand_path("../../../vendor/rubocop-rspec/spec", __dir__)

  def self.load_vendor_spec(example_group, relative_path, pending: [])
    VendorSpecHelper.load_vendor_spec(
      example_group, relative_path, pending: pending, root: VENDOR_SPEC_ROOT
    )
  end
end
