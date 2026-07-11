# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::OrderedMagicComments, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/ordered_magic_comments_spec.rb")
end
