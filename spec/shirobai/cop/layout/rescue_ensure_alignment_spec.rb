# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::RescueEnsureAlignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/rescue_ensure_alignment_spec.rb")
end
