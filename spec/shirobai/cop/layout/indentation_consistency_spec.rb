# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::IndentationConsistency, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/indentation_consistency_spec.rb")
end
