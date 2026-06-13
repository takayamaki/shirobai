# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::FirstHashElementIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/first_hash_element_indentation_spec.rb")

  # The vendor spec never corrects a multiline first pair whose value contains a
  # string spanning lines. Stock passes the pair NODE to `AlignmentCorrector`
  # (when the value begins on the same line as the key), whose
  # `inside_string_ranges` keeps string-interior lines untouched; these
  # examples (expectations taken from a stock run) guard the wrapper's node
  # resolution.
  context "when the first pair value is a multiline string (shirobai extra)" do
    it "does not indent the string-interior line" do
      expect_offense(<<~RUBY)
        a = {
        x: "multi
        ^^^^^^^^^ Use 2 spaces for indentation in a hash, relative to the start of the line where the left curly brace is.
        line"
        }
      RUBY

      expect_correction(<<~RUBY)
        a = {
          x: "multi
        line"
        }
      RUBY
    end
  end
end
