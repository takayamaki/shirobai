# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::MultipleSubjects, :config do
  RSpecVendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/rspec/multiple_subjects_spec.rb"
  )
end
