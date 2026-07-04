# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::DuplicateMethods, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/duplicate_methods_spec.rb")
end
