# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::MultipleMemoizedHelpers, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/multiple_memoized_helpers_spec.rb"
  )
end
