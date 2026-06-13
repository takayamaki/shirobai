# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::AbcSize, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/abc_size_spec.rb")
end
