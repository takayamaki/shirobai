# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceInsideHashLiteralBraces, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_inside_hash_literal_braces_spec.rb")
end
