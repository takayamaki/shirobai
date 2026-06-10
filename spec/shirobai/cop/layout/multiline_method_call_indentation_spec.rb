# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/multiline_method_call_indentation_spec.rb")
end
