# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::ParenthesesAsGroupedExpression, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/parentheses_as_grouped_expression_spec.rb")
end
