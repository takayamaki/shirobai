# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::HashAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/hash_alignment_spec.rb")
end
