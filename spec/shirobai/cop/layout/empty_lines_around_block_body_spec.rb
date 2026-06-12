# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundBlockBody, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_lines_around_block_body_spec.rb")
end
