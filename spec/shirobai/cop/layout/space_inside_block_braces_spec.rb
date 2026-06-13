# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideBlockBraces, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_block_braces_spec.rb")
end
