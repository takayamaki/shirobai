# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Metrics/ClassLength` quirks the vendor spec
# does not exercise, all found by stock probing (2026-07-02):
#
# - Stock's `classlike_code_length` samples line relevance with an off-by-one:
#   `body_line_numbers` are 1-BASED numbers but `@processed_source[n]` indexes
#   the 0-BASED `lines` array, so each number's relevance is read one line
#   below it. A class whose first body line is blank counts one MORE than the
#   body's true relevant-line count, and a comment-only class counts 1 (the
#   `end` line) at `Max: 0`.
# - `class << self` fires inside a module (only `:class` ancestors suppress
#   it), inside a `Class.new` block, and nested in another sclass.
# - `on_casgn` fires for scoped constants, `||=` / `&&=` / `+=` forms and
#   constructor arguments (`class_definition?` allows all of these), and for
#   a `class << self` expression even inside a class.
# - With `CountAsOne: ['method_call']` the constructor form folds the
#   multi-line `Struct.new(...)` send part (stock's fold walk starts at the
#   casgn and reaches the block's `send` child).
# - A multi-line superclass expression's continuation lines count as body.
RSpec.describe Shirobai::Cop::Metrics::ClassLength do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Metrics::ClassLength,
    Shirobai::Cop::Metrics::ClassLength
  ]

  def config_for(max: 5, count_as_one: [])
    RuboCop::Config.new(
      "Metrics/ClassLength" => {
        "Max" => max,
        "Enabled" => true,
        "CountComments" => false,
        "CountAsOne" => count_as_one,
        "Exclude" => []
      }
    )
  end

  let(:config) { config_for }

  it "counts one extra line when the first body line is blank (stock's sampling shift)" do
    # True body relevance is 5 lines; stock reports [6/5] because relevance
    # is sampled one line below each body line number.
    expect_lint_parity(*klasses, <<~RUBY, config)
      class Test

        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
      end
    RUBY
  end

  it "counts a comment-only class as 1 at Max 0 (the `end` line is sampled)" do
    expect_lint_parity(*klasses, <<~RUBY, config_for(max: 0))
      class Test
        # c1
        # c2
      end
    RUBY
  end

  it "shifts the inner-class subtraction window too (blank line after inner `end`)" do
    # Stock reports [7/5]: the number right after the inner class's range
    # samples the line below the blank.
    expect_lint_parity(*klasses, <<~RUBY, config)
      class Outer
        class Inner
          x = 1
        end

        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
        a = 6
      end
    RUBY
  end

  it "samples heredoc content lines per line (blank / comment-looking lines skipped)" do
    expect_lint_parity(*klasses, <<~RUBY, config_for(max: 4))
      class Test
        x = <<~H
          a

          # looks like comment
          b
        H
        y = 1
      end
    RUBY
  end

  it "counts a multi-line superclass expression's continuation lines" do
    expect_lint_parity(*klasses, <<~RUBY, config_for(max: 4))
      class A < Struct.new(:a,
                           :b,
                           :c)
        x = 1
        x = 2
        x = 3
      end
    RUBY
  end

  it "fires on a `class << self` inside a module" do
    expect_lint_parity(*klasses, <<~RUBY, config)
      module M
        class << self
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
        end
      end
    RUBY
  end

  it "fires on both the constructor block and the `class << self` inside it" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = Class.new do
        class << self
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
        end
      end
    RUBY
    expect(offenses.size).to eq(2)
  end

  it "fires on both nested sclasses (`class << self` inside `class << self`)" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      class << self
        class << self
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
        end
      end
    RUBY
    expect(offenses.size).to eq(2)
  end

  it "reports a single offense for `Foo = class << self` at the toplevel (range dedup)" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = class << self
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

  it "fires on a `FOO = class << self` nested inside a class (casgn path has no ancestor check)" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      class Outer
        FOO = class << self
          a = 1
          a = 2
          a = 3
          a = 4
          a = 5
          a = 6
        end
      end
    RUBY
    expect(offenses.size).to eq(2)
  end

  it "fires on a scoped-constant assignment (`Foo::Bar = Class.new do`)" do
    expect_lint_parity(*klasses, <<~RUBY, config)
      Foo::Bar = Class.new do
        a = 1
        a = 2
        a = 3
        a = 4
        a = 5
        a = 6
      end
    RUBY
  end

  it "fires on `&&=` / `+=` assignments and constructor arguments" do
    ["Foo &&= Class.new do", "Foo += Class.new do", "Foo = Class.new(1) do"].each do |head|
      expect_lint_parity(*klasses, <<~RUBY, config)
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

  it "reports a single offense for `Foo = class Bar` (casgn duplicate of on_class)" do
    offenses = expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = class Bar
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

  it "folds the multi-line constructor send part under CountAsOne method_call" do
    # 6 body lines fold to 5 (the two-line `Struct.new(...)` send counts 1):
    # silent at Max 5, [5/4] at Max 4 — both sides must agree.
    source = <<~RUBY
      Foo = Struct.new(:a,
                       :b) do
        x = 1
        x = 2
        x = 3
        x = 4
        x = 5
        x = 6
      end
    RUBY
    expect_lint_parity(*klasses, source, config_for(max: 5, count_as_one: ["method_call"]),
                       expect_offenses: false)
    expect_lint_parity(*klasses, source, config_for(max: 4, count_as_one: ["method_call"]))
  end

  it "counts nested class lines inside a constructor block (body-based measure)" do
    expect_lint_parity(*klasses, <<~RUBY, config)
      Foo = Class.new do
        class Bar
          y = 1
          y = 2
        end
        x = 1
        x = 2
      end
    RUBY
  end

  it "stays silent on a namespace-only class body" do
    expect_lint_parity(*klasses, <<~RUBY, config, expect_offenses: false)
      class C
        module M
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

  it "stays silent on odd constant assignments (no crash)" do
    [
      "X = Y = Z = do_something\n",
      "for FOO in [1] do end\n",
      "begin\nrescue => FOO\nend\n",
      "Foo = Class.new(&blk)\n"
    ].each do |source|
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
    end
  end
end
