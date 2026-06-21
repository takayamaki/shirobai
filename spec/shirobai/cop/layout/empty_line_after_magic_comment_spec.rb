# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::EmptyLineAfterMagicComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/empty_line_after_magic_comment_spec.rb")
end
