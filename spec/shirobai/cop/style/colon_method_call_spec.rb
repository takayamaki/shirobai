# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::ColonMethodCall, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/colon_method_call_spec.rb")
end
