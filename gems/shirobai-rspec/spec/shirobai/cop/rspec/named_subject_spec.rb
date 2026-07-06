# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::RSpec::NamedSubject, :config do
  RSpecVendorSpecHelper.load_vendor_spec(self, "rubocop/cop/rspec/named_subject_spec.rb")
end
