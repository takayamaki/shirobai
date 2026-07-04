# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideParens, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_parens_spec.rb")
end
