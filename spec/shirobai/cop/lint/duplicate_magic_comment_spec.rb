# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::DuplicateMagicComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/duplicate_magic_comment_spec.rb")
end
