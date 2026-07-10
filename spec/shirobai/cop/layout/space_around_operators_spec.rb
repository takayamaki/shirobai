# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceAroundOperators, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_around_operators_spec.rb")
end
