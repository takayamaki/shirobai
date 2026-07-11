# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::EmptyLiteral, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/empty_literal_spec.rb")
end
