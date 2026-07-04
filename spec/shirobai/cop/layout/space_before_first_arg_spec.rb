# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceBeforeFirstArg, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_before_first_arg_spec.rb")
end
