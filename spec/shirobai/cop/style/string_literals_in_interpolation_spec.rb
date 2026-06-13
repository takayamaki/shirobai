# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::StringLiteralsInInterpolation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/string_literals_in_interpolation_spec.rb")
end
