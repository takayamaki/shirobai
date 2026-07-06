# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::MetadataStyle, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/metadata_style_spec.rb"
  )
end
