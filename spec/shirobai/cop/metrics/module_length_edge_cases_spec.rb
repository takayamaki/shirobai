# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Metrics/ModuleLength` quirks the vendor spec
# does not exercise, all found by stock probing (2026-07-02):
#
# - The same line-sampling off-by-one as `ClassLength` (a blank first body
#   line still counts).
# - The cop-local `module_definition?` pattern is much narrower than
#   `ClassLength`'s `class_definition?`: the casgn scope must be nil, the
#   `Module.new` send takes NO arguments (empty parens still match), and
#   `||=` / `&&=` / masgn forms never fire.
RSpec.describe Shirobai::Cop::Metrics::ModuleLength do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::ModuleLength,
    Shirobai::Cop::Metrics::ModuleLength
  ]

  let(:config) do
    RuboCop::Config.new(
      "Metrics/ModuleLength" => {
        "Max" => 5,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => [],
        "Exclude" => []
      }
    )
  end

  it "counts one extra line when the first body line is blank (stock's sampling shift)" do
    expect_lint_parity(*klasses, <<~RUBY, config)
      module Test

        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
      end
    RUBY
  end

  it "fires on `Foo = Module.new() do` (empty parens still match the no-arg pattern)" do
    expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = Module.new() do
        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
        a = 6
      end
    RUBY
  end

  it "reports the inner constant name for a chained `Foo = Bar = Module.new do`" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = Bar = Module.new do
        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
        a = 6
      end
    RUBY
    expect(offenses.size).to eq(1)
  end

  it "stays silent on the casgn forms the cop-local pattern excludes" do
    [
      "Foo::Bar = Module.new do",  # scoped constant
      "Foo = Module.new(1) do",    # constructor argument
      "Foo ||= Module.new do",     # or-assignment
      "Foo &&= Module.new do",     # and-assignment
      "Foo, Bar = Module.new do"   # masgn
    ].each do |head|
      expect_lint_parity(*klasses, <<~RUBY, config, expect_offenses: false)
        #{head}
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
        end
      RUBY
    end
  end

  it "stays silent on a namespace-only module body" do
    expect_lint_parity(*klasses, <<~RUBY, config, expect_offenses: false)
      module M
        class C
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
          a = 7
        end
      end
    RUBY
  end
end
