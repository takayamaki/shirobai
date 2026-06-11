# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Naming::PredicatePrefix, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/naming/predicate_prefix_spec.rb")
end
