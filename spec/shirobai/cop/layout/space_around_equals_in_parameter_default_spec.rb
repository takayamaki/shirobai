# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceAroundEqualsInParameterDefault, :config do
  VendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/layout/space_around_equals_in_parameter_default_spec.rb"
  )
end
