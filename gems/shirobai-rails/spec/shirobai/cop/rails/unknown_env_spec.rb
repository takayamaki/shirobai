# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::UnknownEnv, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/unknown_env_spec.rb")
end
