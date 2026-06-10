# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Naming::MethodName, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/naming/method_name_spec.rb")
end
