# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::SpaceBeforeComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/space_before_comment_spec.rb")
end
