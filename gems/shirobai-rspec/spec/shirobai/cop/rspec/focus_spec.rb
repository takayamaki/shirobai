# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::Focus, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/focus_spec.rb"
  )
end
