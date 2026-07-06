# frozen_string_literal: true

# Spec helper for the shirobai-rails gem. Reuses the core suite's helper
# (vendor spec loader, EdgeCaseParity, RSpec config) and adds:
#
# - the plugin gem itself (which requires rubocop-rails first and replaces
#   the stock Rails cops — see lib/shirobai-rails.rb)
# - the vendor spec root for the rubocop-rails submodule
# - rubocop-rails's `:rails*` shared contexts, which bind the `rails_version`
#   let the vendor Application* specs use. RuboCop's own `:config` shared
#   context reads `rails_version` and mocks `config.gem_versions_in_target`
#   (`railties => <version>`), which is what the `requires_gem` /
#   `TargetRailsVersion` gate on the wrappers consults.
#
# rubocop-rails's config/default.yml is merged into
# `RuboCop::ConfigLoader.default_configuration` automatically:
# `rubocop/rspec/support`'s CopHelper `before(:all)` integrates every
# activated gem with `default_lint_roller_plugin` metadata, and this
# suite's bundle (gems/shirobai-rails/Gemfile) activates rubocop-rails.
require_relative "../../../spec/spec_helper"
require "shirobai-rails"

# Upstream `:rails42` / `:rails50` / ... shared contexts, pulled from the
# pinned submodule so vendor specs run against exactly the version gate they
# were written for.
require_relative "../../../vendor/rubocop-rails/spec/support/shared_contexts"

RSpec.configure do |config|
  # Bind `rails_version` from the `:rails42` / `:rails50` / ... group tags the
  # vendor Application* specs use. The core suite globally includes RuboCop's
  # CopHelper (which defines `let(:rails_version) { false }`), so we include
  # the version contexts explicitly here — the later include wins the `let`,
  # setting `rails_version` for the tagged groups (RuboCop's `:config` context
  # then mocks `gem_versions_in_target` from it, which drives the
  # `TargetRailsVersion` gate on the wrappers).
  {
    rails42: "with Rails 4.2", rails50: "with Rails 5.0", rails51: "with Rails 5.1",
    rails52: "with Rails 5.2", rails60: "with Rails 6.0", rails61: "with Rails 6.1",
    rails70: "with Rails 7.0", rails71: "with Rails 7.1", rails72: "with Rails 7.2",
    rails80: "with Rails 8.0", rails81: "with Rails 8.1"
  }.each do |tag, context_name|
    config.include_context context_name, tag
  end
end

module RailsVendorSpecHelper
  VENDOR_SPEC_ROOT = File.expand_path("../../../vendor/rubocop-rails/spec", __dir__)

  def self.load_vendor_spec(example_group, relative_path, pending: [])
    VendorSpecHelper.load_vendor_spec(
      example_group, relative_path, pending: pending, root: VENDOR_SPEC_ROOT
    )
  end
end
