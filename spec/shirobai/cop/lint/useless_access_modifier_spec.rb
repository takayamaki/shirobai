# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::UselessAccessModifier, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/useless_access_modifier_spec.rb")
end
