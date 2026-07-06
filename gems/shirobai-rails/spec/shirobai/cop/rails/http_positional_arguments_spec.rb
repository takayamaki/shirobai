# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::HttpPositionalArguments, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/http_positional_arguments_spec.rb")
end
