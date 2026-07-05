# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::ArgumentsForwarding, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/arguments_forwarding_spec.rb")
end
