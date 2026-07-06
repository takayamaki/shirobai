# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::SortMetadata, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/sort_metadata_spec.rb"
  )
end
