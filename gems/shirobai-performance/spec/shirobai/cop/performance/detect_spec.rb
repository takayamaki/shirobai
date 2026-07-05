# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Performance::Detect, :config do
  PerformanceVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/performance/detect_spec.rb")
end
