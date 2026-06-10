# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::BlockNesting, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/block_nesting_spec.rb")
end
