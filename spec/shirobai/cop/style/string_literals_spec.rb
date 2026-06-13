# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::StringLiterals, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/string_literals_spec.rb")
end
