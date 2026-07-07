# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for Rails/Pluck. The vendor spec covers the
# canonical map/collect matrix and the Rails version gate; these pin quirks
# probed against stock rubocop-rails 2.35.5 that the Rust port could regress:
#
# - **numblock**: `users.map { _1[:name] }` fires and corrects.
# - **itblock**: `users.map { it[:name] }` fires and corrects.
# - **ancestor block with receiver**: `n.each { x.map { |a| a[:key] } }`
#   does NOT fire (N+1 query guard).
# - **block argument shadowing**: `x.map { |x| x[x] }` does NOT fire.
# - **regexp key**: `x.map { |e| e[/pattern/] }` does NOT fire.
# - **CRLF source**: the wrapper falls back to its standalone entry point
#   so offsets still line up with parser-gem's index.
RSpec.describe "Rails/Pluck edge cases" do
  include EdgeCaseParity

  let(:config) do
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Rails/Pluck"] = (hash["Rails/Pluck"] || {}).merge("Enabled" => true)
    RuboCop::Config.new(hash, default.loaded_path)
  end
  let(:cops) { [RuboCop::Cop::Rails::Pluck, Shirobai::Cop::Rails::Pluck] }

  describe "numblock variant" do
    it "fires and corrects `users.map { _1[:name] }`" do
      corrected = expect_autocorrect_parity(*cops, "users.map { _1[:name] }\n", config)
      expect(corrected).to eq("users.pluck(:name)\n")
    end
  end

  describe "itblock variant" do
    # Stock uses parser-gem which treats `it` as a method call, not an
    # itblock parameter, so parity cannot be tested differentially. The
    # vendor spec (`:ruby34, unsupported_on: :parser`) guards this; we
    # test the shirobai cop alone.
    it "fires and corrects `users.map { it[:name] }`" do
      shiro_offenses, shiro_corrected = autocorrect_run(
        Shirobai::Cop::Rails::Pluck, "users.map { it[:name] }\n", config
      )
      expect(shiro_offenses).not_to be_empty
      expect(shiro_corrected).to eq("users.pluck(:name)\n")
    end
  end

  describe "ancestor block with receiver guard" do
    it "does not fire when map is inside a repeatable block" do
      src = <<~RUBY
        n.each do |x|
          x.map { |a| a[:foo] }
        end
      RUBY
      expect_lint_parity(*cops, src, config, expect_offenses: false)
    end

    it "fires when the outer block has no receiver" do
      src = <<~RUBY
        foo do
          x.map { |a| a[:foo] }
        end
      RUBY
      expect_lint_parity(*cops, src, config)
    end

    it "does not fire inside a numblock with receiver" do
      src = <<~RUBY
        n.each do
          _1.map { |a| a[:foo] }
        end
      RUBY
      expect_lint_parity(*cops, src, config, expect_offenses: false)
    end
  end

  describe "block argument shadowing" do
    it "does not fire when key IS the block argument" do
      expect_lint_parity(*cops, "x.map { |x| x[x] }\n", config, expect_offenses: false)
    end

    it "does not fire when key CONTAINS the block argument" do
      expect_lint_parity(*cops, "x.map { |a| a[foo...a.to_something] }\n", config,
                         expect_offenses: false)
    end
  end

  describe "regexp key" do
    it "does not fire for regexp keys" do
      expect_lint_parity(*cops, "x.map { |a| a[/pattern/] }\n", config, expect_offenses: false)
    end
  end

  describe "safe navigation" do
    it "fires and corrects `x&.map { |a| a[:foo] }`" do
      corrected = expect_autocorrect_parity(*cops, "x&.map { |a| a[:foo] }\n", config)
      expect(corrected).to eq("x&.pluck(:foo)\n")
    end
  end

  describe "collect alias" do
    it "fires and corrects `x.collect { |a| a[:foo] }`" do
      corrected = expect_autocorrect_parity(*cops, "x.collect { |a| a[:foo] }\n", config)
      expect(corrected).to eq("x.pluck(:foo)\n")
    end
  end

  describe "string key" do
    it "fires and corrects string literal keys" do
      corrected = expect_autocorrect_parity(*cops, "x.map { |a| a['foo'] }\n", config)
      expect(corrected).to eq("x.pluck('foo')\n")
    end
  end

  describe "method call key" do
    it "fires and corrects method call keys" do
      corrected = expect_autocorrect_parity(
        *cops, "x.map { |a| a[obj.do_something] }\n", config
      )
      expect(corrected).to eq("x.pluck(obj.do_something)\n")
    end
  end

  describe "CRLF source" do
    it "keeps offsets aligned on the standalone fallback" do
      expect_autocorrect_parity(*cops, "x.map { |a| a[:foo] }\r\n", config)
    end
  end
end
