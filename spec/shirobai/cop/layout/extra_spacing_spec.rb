# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::ExtraSpacing, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/extra_spacing_spec.rb")
end
