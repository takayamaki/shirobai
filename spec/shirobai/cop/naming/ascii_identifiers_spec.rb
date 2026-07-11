# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Naming::AsciiIdentifiers, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/naming/ascii_identifiers_spec.rb")
end
