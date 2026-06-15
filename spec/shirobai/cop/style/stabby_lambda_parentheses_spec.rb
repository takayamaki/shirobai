# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::StabbyLambdaParentheses, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/stabby_lambda_parentheses_spec.rb")
end
