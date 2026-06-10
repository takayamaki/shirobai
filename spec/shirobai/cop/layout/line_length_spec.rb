# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::LineLength, :config do
  # The detection logic (offense location for over-long lines and every
  # non-autocorrect option) is fully ported to Rust. Auto-correction (splitting
  # long strings / breaking arrays, hashes and method calls across lines) is not
  # yet ported, so every example under the `autocorrection` context is pending.
  # Helper used by several upstream examples to embed a single trailing space
  # without it being stripped from the heredoc source.
  let(:trailing_whitespace) { " " }

  VendorSpecHelper.load_vendor_spec(self, "rubocop/cop/layout/line_length_spec.rb")
end
