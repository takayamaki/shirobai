# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideReferenceBrackets, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_reference_brackets_spec.rb")
end
