# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Metrics/BlockNesting` quirks the vendor spec
# does not exercise, all found by stock probing (2026-11-01):
#
# - A modifier rescue (`x rescue y`) is a `:rescue` node with a `:resbody`
#   child in parser, and `:resbody` is a nesting block. So the `rescue y`
#   part adds a level and is reported at the `rescue` keyword. Prism gives a
#   separate `RescueModifierNode`, so it must be counted on its own.
#   Found in Ruby's stdlib `tempfile.rb`
#   (`unlinked = File.unlink tmpfile.path rescue nil`).
# - Only the rescue value is inside the `:resbody`. The rescued expression is
#   a sibling at the outer level.
# - Chained modifier rescues (`a rescue b rescue c`) are siblings, not nested.
RSpec.describe Shirobai::Cop::Metrics::BlockNesting do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::BlockNesting,
    Shirobai::Cop::Metrics::BlockNesting
  ]

  def config_for(max: 2)
    RuboCop::Config.new(
      "Metrics/BlockNesting" => {
        "Max" => max,
        "Enabled" => true,
        "CountBlocks" => false,
        "CountModifierForms" => false
      }
    )
  end

  describe "modifier rescue" do
    context "when a modifier rescue in an assignment goes over Max" do
      it "reports the rescue part like stock" do
        source = <<~RUBY
          if a
            if b
              x = y rescue nil
            end
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for)
      end
    end

    context "when the rescue value nests another construct" do
      it "reports only the rescue part like stock" do
        source = <<~RUBY
          if a
            if b
              y rescue (if c
                then d
              end)
            end
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for)
      end
    end

    context "when the rescued expression nests another construct" do
      it "keeps the rescued expression at the outer level like stock" do
        source = <<~RUBY
          if a
            if b
              (if c
                then d
              end) rescue nil
            end
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for)
      end
    end

    context "when modifier rescues are chained" do
      it "treats each rescue part as a sibling like stock" do
        source = <<~RUBY
          if a
            if b
              y rescue z rescue w
            end
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for)
      end
    end

    context "when the modifier rescue is inside a reported rescue clause" do
      it "does not report it again like stock" do
        source = <<~RUBY
          if a
            if b
              begin
                y
              rescue
                z rescue w
              end
            end
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for)
      end
    end

    context "when the modifier rescue stays within Max" do
      it "reports nothing like stock" do
        source = <<~RUBY
          if a
            x = y rescue nil
          end
        RUBY
        expect_lint_parity(*klasses, source, config_for, expect_offenses: false)
      end
    end
  end
end
