# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::Debugger, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/debugger_spec.rb")
end
