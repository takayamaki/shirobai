# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::DescribedClass, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/described_class_spec.rb"
  )
end
