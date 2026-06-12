# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundModuleBody, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_lines_around_module_body_spec.rb")
end
