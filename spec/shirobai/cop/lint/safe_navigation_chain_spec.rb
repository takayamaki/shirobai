# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::SafeNavigationChain, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/safe_navigation_chain_spec.rb")
end
