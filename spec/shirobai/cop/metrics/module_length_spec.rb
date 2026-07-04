# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::ModuleLength, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/module_length_spec.rb")
end
