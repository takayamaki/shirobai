# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::Pluck, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/pluck_spec.rb")
end
