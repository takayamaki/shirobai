# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideArrayLiteralBrackets, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_array_literal_brackets_spec.rb")
end
