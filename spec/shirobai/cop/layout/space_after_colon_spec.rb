# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceAfterColon, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_after_colon_spec.rb")
end
