# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::RedundantSelf, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/redundant_self_spec.rb")
end
