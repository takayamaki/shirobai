# frozen_string_literal: true

require "spec_helper"

# Shared examples (`condition modifier cop`) used by the vendor spec.
require_relative "../../../../vendor/rubocop/spec/support/condition_modifier_cop"

RSpec.describe Shirobai::Cop::Style::IfUnlessModifier, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/if_unless_modifier_spec.rb")
end
