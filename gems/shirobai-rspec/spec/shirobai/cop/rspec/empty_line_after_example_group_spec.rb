# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterExampleGroup, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/empty_line_after_example_group_spec.rb")
end
