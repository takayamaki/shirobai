# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLineAfterGuardClause, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_line_after_guard_clause_spec.rb")
end
