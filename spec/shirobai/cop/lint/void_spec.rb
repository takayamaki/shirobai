# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::Void, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/void_spec.rb")
end
