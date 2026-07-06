# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterExample, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/empty_line_after_example_spec.rb")
end
