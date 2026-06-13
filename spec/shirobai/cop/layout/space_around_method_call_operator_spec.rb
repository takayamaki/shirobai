# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceAroundMethodCallOperator, :config do
  VendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/layout/space_around_method_call_operator_spec.rb"
  )
end
