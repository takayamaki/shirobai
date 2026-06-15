# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::AccessModifierIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/access_modifier_indentation_spec.rb")
end
