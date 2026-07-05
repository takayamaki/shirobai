# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::RedundantFreeze, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/redundant_freeze_spec.rb")
end
