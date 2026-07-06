# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for Rails/DynamicFindBy. The vendor spec covers
# the canonical finder matrix, the AllowedMethods / AllowedReceivers /
# Whitelist suppressions and the receiver/no-receiver split; these pin quirks
# probed against stock rubocop-rails 2.35.5 that the Rust port could regress:
#
# - **csend**: `user&.find_by_name(name)` fires and corrects (on_csend alias).
# - **each_ancestor(:class).any?**: a receiverless finder fires when ANY
#   enclosing class inherits ActiveRecord, even a nested non-AR class inside
#   an AR class.
# - **block-pass is a parser argument**: `find_by_name(&blk)` counts `&blk` as
#   one argument (matches the keyword count) and fires.
# - **lone bang column**: `find_by_!(x)` captures `!` as the column (lazy
#   group), so `static_name` stays `find_by`.
# - **CRLF**: the wrapper falls back to its standalone entry point so offsets
#   still line up with parser-gem's index.
RSpec.describe "Rails/DynamicFindBy edge cases" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }
  let(:cops) { [RuboCop::Cop::Rails::DynamicFindBy, Shirobai::Cop::Rails::DynamicFindBy] }

  describe "safe navigation" do
    it "fires and corrects `user&.find_by_name(name)`" do
      corrected = expect_autocorrect_parity(*cops, "user&.find_by_name(name)\n", config)
      expect(corrected).to eq("user&.find_by(name: name)\n")
    end
  end

  describe "receiverless inside an ActiveRecord class" do
    it "fires when a nested non-AR class is inside an AR class" do
      src = <<~RUBY
        class Outer < ApplicationRecord
          class Inner
            def m
              find_by_name(name)
            end
          end
        end
      RUBY
      expect_lint_parity(*cops, src, config)
    end

    it "does not fire when no ancestor class inherits ActiveRecord" do
      src = <<~RUBY
        class Outer
          class Inner < Foo
            def m
              find_by_name(name)
            end
          end
        end
      RUBY
      expect_lint_parity(*cops, src, config, expect_offenses: false)
    end
  end

  describe "block-pass argument" do
    it "counts `&blk` as one argument and corrects" do
      expect_autocorrect_parity(*cops, "User.find_by_name(&blk)\n", config)
    end
  end

  describe "lone bang column" do
    it "keeps `find_by` (the `!` is the lazy column capture)" do
      expect_autocorrect_parity(*cops, "User.find_by_!(x)\n", config)
    end
  end

  describe "multiline arguments" do
    it "inserts each keyword before its argument across lines" do
      src = "User.find_by_name_and_email_and_token(\n  name,\n  email,\n  token\n)\n"
      expect_autocorrect_parity(*cops, src, config)
    end
  end

  describe "CRLF source" do
    it "keeps offsets aligned on the standalone fallback" do
      expect_autocorrect_parity(*cops, "User.find_by_name(name)\r\n", config)
    end
  end

  describe "no offense for splat / hash arguments" do
    it "matches stock for splat and trailing hash" do
      expect_lint_parity(*cops, "User.find_by_scan(*args)\n", config, expect_offenses: false)
      expect_lint_parity(*cops, "Post.find_by_title_and_id(\"foo\", limit: 1)\n", config,
                         expect_offenses: false)
    end
  end
end
