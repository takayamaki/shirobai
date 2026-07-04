# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceBeforeBlockBraces, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_before_block_braces_spec.rb")
end
