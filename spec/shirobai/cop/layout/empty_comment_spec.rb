# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_comment_spec.rb")
end
