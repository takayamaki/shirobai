# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::DirectiveScope, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/directive_scope_spec.rb")
end
