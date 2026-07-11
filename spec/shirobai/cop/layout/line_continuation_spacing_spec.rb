# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::LineContinuationSpacing, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/line_continuation_spacing_spec.rb")
end
