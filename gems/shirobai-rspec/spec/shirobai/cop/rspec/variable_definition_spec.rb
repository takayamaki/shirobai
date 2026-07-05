# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::VariableDefinition, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/variable_definition_spec.rb")
end
