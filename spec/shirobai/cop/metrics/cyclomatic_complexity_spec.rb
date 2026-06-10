# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::CyclomaticComplexity, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/cyclomatic_complexity_spec.rb")
end
