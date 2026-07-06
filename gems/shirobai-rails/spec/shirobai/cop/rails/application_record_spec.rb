# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::ApplicationRecord, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/application_record_spec.rb")
end
