# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Performance::TimesMap, :config do
  PerformanceVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/performance/times_map_spec.rb")
end
