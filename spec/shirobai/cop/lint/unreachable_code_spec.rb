# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::UnreachableCode, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/unreachable_code_spec.rb")
end
