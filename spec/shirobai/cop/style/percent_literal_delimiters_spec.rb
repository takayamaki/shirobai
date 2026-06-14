# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::PercentLiteralDelimiters, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/percent_literal_delimiters_spec.rb")
end
