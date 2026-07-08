# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::SharedExamples, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/shared_examples_spec.rb"
  )
end
