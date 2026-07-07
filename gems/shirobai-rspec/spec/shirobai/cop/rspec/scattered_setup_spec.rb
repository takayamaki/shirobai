# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::ScatteredSetup, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/scattered_setup_spec.rb"
  )
end
