# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::ElseAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/else_alignment_spec.rb")
end
