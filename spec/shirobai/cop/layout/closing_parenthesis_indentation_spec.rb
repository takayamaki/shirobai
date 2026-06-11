# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::ClosingParenthesisIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/closing_parenthesis_indentation_spec.rb")
end
