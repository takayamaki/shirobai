# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::TrailingEmptyLines, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/trailing_empty_lines_spec.rb")
end
