# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::FrozenStringLiteralComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/frozen_string_literal_comment_spec.rb")
end
