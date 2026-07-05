# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::FileNull, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/file_null_spec.rb")
end
