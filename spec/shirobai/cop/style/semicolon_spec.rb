# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::Semicolon, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/semicolon_spec.rb")
end
