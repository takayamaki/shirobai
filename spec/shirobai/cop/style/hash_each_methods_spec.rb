# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::HashEachMethods, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/hash_each_methods_spec.rb")
end
