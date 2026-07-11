# frozen_string_literal: true

require "spec_helper"
require_relative "../../../../vendor/rubocop/spec/support/encoding_helper"

RSpec.describe Shirobai::Cop::Layout::EndOfLine, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/end_of_line_spec.rb")
end
