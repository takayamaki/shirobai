# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::RequireParentheses, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/require_parentheses_spec.rb")
end
