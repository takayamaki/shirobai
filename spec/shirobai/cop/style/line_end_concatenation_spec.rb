# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::LineEndConcatenation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/line_end_concatenation_spec.rb")
end
