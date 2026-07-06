# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::DeprecatedActiveModelErrorsMethods, :config do
  RailsVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rails/deprecated_active_model_errors_methods_spec.rb"
  )
end
