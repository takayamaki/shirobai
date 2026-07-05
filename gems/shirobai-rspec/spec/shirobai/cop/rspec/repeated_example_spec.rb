# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::RepeatedExample, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/repeated_example_spec.rb"
  )
end
