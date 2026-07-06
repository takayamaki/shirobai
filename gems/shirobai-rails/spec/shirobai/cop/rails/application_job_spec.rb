# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Rails::ApplicationJob, :config do
  RailsVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rails/application_job_spec.rb")
end
