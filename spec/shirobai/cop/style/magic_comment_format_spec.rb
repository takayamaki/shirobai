# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Style::MagicCommentFormat, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/style/magic_comment_format_spec.rb")
end
