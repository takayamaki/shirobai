# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterHook, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/empty_line_after_hook_spec.rb")
end
