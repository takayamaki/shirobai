# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::BlockAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/block_alignment_spec.rb")
end
