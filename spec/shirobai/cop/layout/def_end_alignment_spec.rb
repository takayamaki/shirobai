# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::DefEndAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/def_end_alignment_spec.rb")
end
