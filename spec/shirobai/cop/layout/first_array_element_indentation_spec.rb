# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::FirstArrayElementIndentation, :config do
  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/first_array_element_indentation_spec.rb")

  # The vendor spec never corrects a multiline first element that contains a
  # string spanning lines. Stock passes the element NODE to
  # `AlignmentCorrector`, whose `inside_string_ranges` keeps string-interior
  # lines untouched; these examples (expectations taken from a stock run) guard
  # the wrapper's node resolution.
  context "when the first element is a multiline string (shirobai extra)" do
    it "does not indent the string-interior line" do
      expect_offense(<<~RUBY)
        a = [
        "multi
        ^^^^^^ Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.
        line"
        ]
      RUBY

      expect_correction(<<~RUBY)
        a = [
          "multi
        line"
        ]
      RUBY
    end

    it "indents a nested array's lines but not a string-interior line" do
      expect_offense(<<~RUBY)
        a = [
        [1,
        ^^^ Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is.
         "x
        y"]
        ]
      RUBY

      expect_correction(<<~RUBY)
        a = [
          [1,
           "x
        y"]
        ]
      RUBY
    end
  end
end
