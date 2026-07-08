# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::SubjectStub, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/subject_stub_spec.rb"
  )
end
