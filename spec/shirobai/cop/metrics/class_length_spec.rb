# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Metrics::ClassLength, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/metrics/class_length_spec.rb")
end
