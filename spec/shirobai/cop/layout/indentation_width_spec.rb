# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::IndentationWidth, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/indentation_width_spec.rb")
end
