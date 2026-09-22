# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Lint::MisplacedMagicComment, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/lint/misplaced_magic_comment_spec.rb")
end
