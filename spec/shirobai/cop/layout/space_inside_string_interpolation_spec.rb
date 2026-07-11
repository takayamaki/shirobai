# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideStringInterpolation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_string_interpolation_spec.rb")
end
