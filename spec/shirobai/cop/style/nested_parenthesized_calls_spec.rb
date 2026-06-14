# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::NestedParenthesizedCalls, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/nested_parenthesized_calls_spec.rb")
end
