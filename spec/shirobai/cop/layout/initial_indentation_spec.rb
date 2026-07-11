# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::InitialIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/initial_indentation_spec.rb")
end
