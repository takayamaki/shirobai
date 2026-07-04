# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::ArrayAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/array_alignment_spec.rb")
end
