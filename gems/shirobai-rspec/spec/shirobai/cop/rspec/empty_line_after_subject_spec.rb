# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::EmptyLineAfterSubject, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/empty_line_after_subject_spec.rb")
end
