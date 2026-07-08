# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::Dialect, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/dialect_spec.rb"
  )
end
