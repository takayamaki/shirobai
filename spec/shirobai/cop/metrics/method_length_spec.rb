# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::MethodLength, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/method_length_spec.rb")
end
