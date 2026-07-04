# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Performance::EndWith, :config do
  PerformanceVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/performance/end_with_spec.rb")
end
