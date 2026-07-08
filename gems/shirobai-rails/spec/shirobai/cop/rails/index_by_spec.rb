# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::IndexBy, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/index_by_spec.rb")
end
