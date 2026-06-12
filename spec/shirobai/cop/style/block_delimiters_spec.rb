# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::BlockDelimiters, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/block_delimiters_spec.rb")
end
