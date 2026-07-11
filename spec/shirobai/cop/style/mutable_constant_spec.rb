# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::MutableConstant, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/mutable_constant_spec.rb")
end
