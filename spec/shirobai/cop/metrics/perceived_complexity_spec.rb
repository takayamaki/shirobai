# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::PerceivedComplexity, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/perceived_complexity_spec.rb")
end
