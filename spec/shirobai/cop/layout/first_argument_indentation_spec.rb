# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::FirstArgumentIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/first_argument_indentation_spec.rb")
end
