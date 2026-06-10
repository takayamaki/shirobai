# frozen_string_literal: true

require "rubocop"
require "rubocop/rspec/support"

$LOAD_PATH.unshift File.join(__dir__, "..", "lib")
require "shirobai"

# Vendor spec support helpers used by the upstream cop specs we re-run verbatim
# (e.g. `trailing_whitespace`).
require_relative "../vendor/rubocop/spec/support/misc_helper"
# `strip_margin` is used by some upstream cop specs to build indented fixtures.
require_relative "../vendor/rubocop/spec/core_ext/string"

RSpec.configure do |config|
  config.expect_with :rspec do |expectations|
    expectations.include_chain_clauses_in_custom_matcher_descriptions = true
  end

  config.mock_with :rspec do |mocks|
    mocks.verify_partial_doubles = true
  end

  config.shared_context_metadata_behavior = :apply_to_host_groups
  config.filter_run_when_matching :focus
  config.order = :defined
end

module VendorSpecHelper
  VENDOR_SPEC_ROOT = File.expand_path("../vendor/rubocop/spec", __dir__)

  def self.load_vendor_spec(example_group, relative_path, pending: [])
    full_path = File.join(VENDOR_SPEC_ROOT, relative_path)
    source = File.read(full_path)

    first_line = source.index(/^RSpec\.describe\b/)
    raise "Could not find RSpec.describe in #{relative_path}" unless first_line

    header_end = source.index("\n", first_line) + 1
    body = source[header_end..].sub(/\nend\s*\z/, "\n")

    unless pending.empty?
      patterns = Array(pending)
      example_group.before do |example|
        desc = example.full_description
        if patterns.any? { |p| desc.match?(p) }
          skip("staged: not yet ported")
        end
      end
    end

    lineno = source[0...header_end].count("\n") + 1
    example_group.instance_eval(body, full_path, lineno)
  end
end
