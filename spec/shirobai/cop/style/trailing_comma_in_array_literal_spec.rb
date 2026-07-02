# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::TrailingCommaInArrayLiteral, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/trailing_comma_in_array_literal_spec.rb")
end
