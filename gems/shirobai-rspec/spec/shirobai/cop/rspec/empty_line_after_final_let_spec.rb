# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterFinalLet, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/empty_line_after_final_let_spec.rb")
end
