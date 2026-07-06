# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::PendingWithoutReason, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/pending_without_reason_spec.rb"
  )
end
