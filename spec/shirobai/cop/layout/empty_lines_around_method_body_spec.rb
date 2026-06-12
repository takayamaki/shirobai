# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundMethodBody, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_lines_around_method_body_spec.rb")
end
