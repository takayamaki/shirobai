# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::DotPosition, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/dot_position_spec.rb")
end
