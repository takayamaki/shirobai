# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::TrailingCommaInArguments, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/trailing_comma_in_arguments_spec.rb")
end
