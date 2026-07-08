# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::IndexWith, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/index_with_spec.rb")
end
