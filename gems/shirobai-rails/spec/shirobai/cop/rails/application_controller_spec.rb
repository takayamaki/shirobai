# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::ApplicationController, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/application_controller_spec.rb")
end
