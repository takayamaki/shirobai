# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::LetSetup, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/let_setup_spec.rb")
end
