# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for the Application* cops (shared
# `EnforceSuperclass` machinery). The vendor spec covers the canonical
# subclass / Class.new matrix; these pin quirks probed against stock
# rubocop-rails 2.35.5 (+ rubocop 1.88.0 + railties 7.0.8,
# `.tmp/2026-07-06`) that a refactor could silently regress:
#
# - **cbase base** (`< ::ActiveRecord::Base`): the offense/replace range
#   includes the leading `::`.
# - **class-name exemption is scope-insensitive**: `Foo::ApplicationRecord`
#   is exempt (terminal name is `ApplicationRecord`).
# - **Class.new exemption**: exempt only when the call is the direct value
#   of a `SUPERCLASS =` write (nil / cbase scope), covering the `do..end` /
#   `{}` block forms. A namespaced casgn is NOT exempt; a cross-cop
#   assignment (`ApplicationController = Class.new(ActiveRecord::Base)`)
#   still fires `Rails/ApplicationRecord`.
# - **arity**: `Class.new(Base, x)` (two args) and `Class.new(Base, &blk)`
#   (block-pass) do not match; a literal block does.
# - **CRLF**: the wrapper falls back to its standalone entry point when
#   `buffer.source != raw_source` (CRLF / BOM), so offsets must still line
#   up with parser-gem's index.
RSpec.describe "Application* cops edge cases" do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }
  let(:record) { [RuboCop::Cop::Rails::ApplicationRecord, Shirobai::Cop::Rails::ApplicationRecord] }
  let(:controller) do
    [RuboCop::Cop::Rails::ApplicationController, Shirobai::Cop::Rails::ApplicationController]
  end

  describe "cbase base const" do
    it "corrects `< ::ActiveRecord::Base` including the leading ::" do
      corrected = expect_autocorrect_parity(*record, "class Foo < ::ActiveRecord::Base\nend\n", config)
      expect(corrected).to eq("class Foo < ApplicationRecord\nend\n")
    end

    it "corrects the Class.new cbase-base form" do
      expect_autocorrect_parity(*record, "Foo = Class.new(::ActiveRecord::Base)\n", config)
    end
  end

  describe "class-name exemption" do
    it "exempts the ApplicationRecord class itself" do
      expect_lint_parity(*record, "class ApplicationRecord < ActiveRecord::Base\nend\n", config,
                         expect_offenses: false)
    end

    it "exempts a namespaced ApplicationRecord name" do
      expect_lint_parity(*record, "class Foo::ApplicationRecord < ActiveRecord::Base\nend\n",
                         config, expect_offenses: false)
    end
  end

  describe "Class.new exemption" do
    it "exempts the direct ApplicationRecord = Class.new(Base) form" do
      expect_lint_parity(*record, "ApplicationRecord = Class.new(ActiveRecord::Base)\n", config,
                         expect_offenses: false)
    end

    it "exempts the do..end block form" do
      expect_lint_parity(*record,
                         "ApplicationRecord = Class.new(ActiveRecord::Base) do\n  def x; end\nend\n",
                         config, expect_offenses: false)
    end

    it "exempts the brace block form" do
      expect_lint_parity(*record, "ApplicationRecord = Class.new(ActiveRecord::Base) { def x; end }\n",
                         config, expect_offenses: false)
    end

    it "exempts the cbase ::ApplicationRecord = ... form" do
      expect_lint_parity(*record, "::ApplicationRecord = Class.new(ActiveRecord::Base)\n", config,
                         expect_offenses: false)
    end

    it "does NOT exempt a namespaced casgn" do
      expect_autocorrect_parity(*record, "Foo::ApplicationRecord = Class.new(ActiveRecord::Base)\n",
                                config)
    end

    it "does NOT exempt a cross-cop assignment (still fires ApplicationRecord)" do
      expect_autocorrect_parity(*record,
                                "ApplicationController = Class.new(ActiveRecord::Base)\n", config)
    end

    it "flags a named Class.new block form" do
      expect_autocorrect_parity(*record,
                                "Foo = Class.new(ActiveRecord::Base) do\n  def x; end\nend\n", config)
    end
  end

  describe "anonymous and nested forms" do
    it "flags an anonymous Class.new with a block" do
      expect_autocorrect_parity(*record, "Class.new(ActiveRecord::Base) {}\n", config)
    end

    it "flags a Class.new nested inside a send argument" do
      expect_autocorrect_parity(*record, "wrap(Class.new(ActiveRecord::Base))\n", config)
    end
  end

  describe "arity gating" do
    it "does not flag Class.new with two positional args" do
      expect_lint_parity(*record, "A = Class.new(ActiveRecord::Base, foo)\n", config,
                         expect_offenses: false)
    end

    it "does not flag Class.new with a block-pass argument" do
      expect_lint_parity(*record, "A = Class.new(ActiveRecord::Base, &blk)\n", config,
                         expect_offenses: false)
    end
  end

  describe "CRLF source (standalone fallback path)" do
    it "matches stock offenses and correction with CRLF line endings" do
      src = "class Foo < ActiveRecord::Base\r\nend\r\n"
      expect_autocorrect_parity(*record, src, config)
    end

    it "matches stock for the controller cop with CRLF" do
      src = "class Foo < ActionController::Base\r\nend\r\n"
      expect_autocorrect_parity(*controller, src, config)
    end
  end
end
