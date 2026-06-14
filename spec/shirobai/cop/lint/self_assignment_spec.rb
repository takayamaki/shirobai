# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::SelfAssignment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/self_assignment_spec.rb")
end
