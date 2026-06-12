# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundBeginBody, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_lines_around_begin_body_spec.rb")
end
