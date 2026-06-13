# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EndAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/end_alignment_spec.rb")
end
