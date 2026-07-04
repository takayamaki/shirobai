# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Performance::StringInclude, :config do
  PerformanceVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/performance/string_include_spec.rb"
  )
end
