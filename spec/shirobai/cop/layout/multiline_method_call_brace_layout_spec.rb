# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallBraceLayout, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/multiline_method_call_brace_layout_spec.rb")
end
