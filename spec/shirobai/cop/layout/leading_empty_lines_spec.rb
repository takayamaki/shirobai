# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::LeadingEmptyLines, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/leading_empty_lines_spec.rb")
end
