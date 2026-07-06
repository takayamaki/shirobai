# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyMetadata, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/empty_metadata_spec.rb"
  )
end
