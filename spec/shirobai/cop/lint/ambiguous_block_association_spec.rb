# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::AmbiguousBlockAssociation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/ambiguous_block_association_spec.rb")
end
