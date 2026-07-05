# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::RepeatedDescription, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/repeated_description_spec.rb"
  )
end
