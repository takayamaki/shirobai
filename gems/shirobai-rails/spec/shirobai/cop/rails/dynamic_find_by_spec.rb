# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::DynamicFindBy, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/dynamic_find_by_spec.rb")
end
