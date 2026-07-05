# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::VariableName, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/variable_name_spec.rb")
end
