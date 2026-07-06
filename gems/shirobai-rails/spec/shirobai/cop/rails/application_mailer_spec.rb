# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::ApplicationMailer, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/application_mailer_spec.rb")
end
