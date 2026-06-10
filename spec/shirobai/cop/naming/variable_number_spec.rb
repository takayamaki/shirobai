# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Naming::VariableNumber, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/naming/variable_number_spec.rb")
end
