# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::HashSyntax, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/hash_syntax_spec.rb")
end
