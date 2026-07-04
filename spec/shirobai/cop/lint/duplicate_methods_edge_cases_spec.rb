# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/DuplicateMethods`.
#
# Quirks probed against stock that the vendor spec does not pin:
#
# - `lookup_constant` failure: `each_ancestor(:class, :module, :casgn)`
#   with a block returns SELF, so an unresolvable `def Const.m` keys on the
#   parser s-expression of the whole defs node — identical bodies collide,
#   different bodies do not.
# - numblock/itblock ancestors are invisible to `parent_module_name`
#   (skipped, not aborting), unlike plain blocks (which abort).
# - only `class_eval` gets the const-receiver treatment; `module_eval` /
#   `class_exec` blocks abort like any other block. A csend
#   (`A&.class_eval`) still counts.
# - attr_* is tracked even inside `if` (stock has no condition check on
#   that path); def/alias/delegator are suppressed.
# - parser appends `&block_pass` as a trailing send argument: `attr :foo,
#   true, &b` has THREE args, so the `true` no longer makes it writable.
# - `Struct.new do ... end` is invisible (not Class/Module).
# - `x = Class.new do ... end` is excluded via the lvasgn parent, but
#   `x = (Class.new do ... end)` is NOT (the parens `begin` breaks the
#   direct-parent check); or-assigns and ivasgns are not excluded either.
# - parser wraps multi-statement bodies in `(begin)`: the anonymous-block
#   scope id is `path:line` for a single-statement def body but nil for a
#   multi-statement one, which changes whether twin anonymous defs collide.
# - a def in the ARGUMENTS of `Class.new(...) do ... end` sits inside the
#   parser block node and joins the anonymous class scope.
# - `for`/`while` bodies are invisible scopes: defs inside them collide
#   with top-level defs.
# - `class << B` (const subject) and `class self::B` produce their probed
#   scope spellings; `class << foo.bar` keys on the subject's selector.
RSpec.describe Shirobai::Cop::Lint::DuplicateMethods do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::DuplicateMethods,
    Shirobai::Cop::Lint::DuplicateMethods
  ]

  describe "sexp-key fallback for unresolvable const receivers" do
    it "flags identical unresolvable defs (same s-expression)" do
      expect_lint_parity(*klasses, "def B.foo\n  body1\nend\ndef B.foo\n  body1\nend\n", config)
    end

    it "does NOT flag unresolvable defs with different bodies" do
      source = "def B.foo\n  body1\nend\ndef B.foo\n  body2\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "prefixes the sexp key with the enclosing def name" do
      expect_lint_parity(*klasses,
                         "def outer\n  def B.foo; end\n  def B.foo; end\nend\n",
                         config)
    end

    it "resolves the const through a casgn Class.new ancestor instead" do
      expect_lint_parity(*klasses,
                         "Foo = Class.new do\n  def Foo.x; end\n  def Foo.x; end\nend\n",
                         config)
    end
  end

  describe "scope-frame quirks" do
    it "sees through numblock ancestors" do
      # The itblock twin is version-dependent (bare `it` is a method call
      # before 3.4, making the block a PLAIN one for stock while prism's
      # Latest grammar always sees an itblock); the vendor spec pins it
      # under `:ruby34`, so only the numblock case is differential here.
      expect_lint_parity(*klasses,
                         "class A\n  foo { _1; def x; end }\n  def x; end\nend\n",
                         config)
    end

    it "aborts at a plain block ancestor" do
      source = "class A\n  foo { def x; end }\n  def x; end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "counts class_eval with a csend receiver" do
      expect_lint_parity(*klasses,
                         "A&.class_eval do\n  def x; end\n  def x; end\nend\n",
                         config)
    end

    it "does not special-case module_eval or class_exec" do
      ["A.module_eval do\n  def x; end\n  def x; end\nend\n",
       "A.class_exec do\n  def x; end\n  def x; end\nend\n"].each do |source|
        expect_lint_parity(*klasses, source, config, expect_offenses: false)
        expect(lint_offenses(klasses.first, source, config)).to be_empty
      end
    end

    it "collides a def inside a bare class_eval with the enclosing class" do
      expect_lint_parity(*klasses,
                         "class A\n  class_eval do\n    def foo; end\n  end\n  def foo; end\nend\n",
                         config)
    end

    it "ignores Struct.new blocks" do
      source = "Foo = Struct.new(:a) do\n  def x; end\n  def x; end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "tracks defs inside for/while bodies at the outer scope" do
      expect_lint_parity(*klasses, "for i in y do\n  def x; end\nend\ndef x; end\n", config)
      expect_lint_parity(*klasses, "while y\n  def x; end\nend\ndef x; end\n", config)
    end

    it "keys `class << B` under the enclosing scope" do
      expect_lint_parity(*klasses,
                         "class A\n  class << B\n    def x; end\n    def x; end\n  end\nend\n",
                         config)
    end

    it "keys `class self::B` with the nil-namespace spelling" do
      expect_lint_parity(*klasses,
                         "class A\n  class self::B\n    def x; end\n    def x; end\n  end\nend\n",
                         config)
    end

    it "keys `class << foo.bar` on the subject selector" do
      expect_lint_parity(*klasses,
                         "class << foo.bar\n  def x; end\n  def x; end\nend\n",
                         config)
    end
  end

  describe "condition handling" do
    it "still tracks attr_* inside a condition" do
      expect_lint_parity(*klasses,
                         "class A\n  if cond\n    attr_reader :foo\n  end\n  def foo; end\nend\n",
                         config)
    end

    it "suppresses def/alias/def_delegator inside a condition" do
      source = "class A\n  def foo; end\n  if cond\n    def foo; end\n    alias foo bar\n" \
               "    def_delegator :x, :foo\n  end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "attr / alias argument quirks" do
    it "treats a block-pass as a trailing attr argument" do
      # `attr :foo, true, &b` has three parser args -> not writable.
      source = "class A\n  attr :foo, true, &b\n  def foo=(v); end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
      # ...while the reader itself still collides.
      expect_lint_parity(*klasses,
                         "class A\n  attr :foo, true, &b\n  def foo; end\nend\n",
                         config)
    end

    it "ignores alias_method with string or extra arguments" do
      source = "class A\n  def foo; end\n  alias_method 'foo', 'bar'\n" \
               "  alias_method :foo, :bar, :baz\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "ignores non-symbol attr arguments" do
      expect_lint_parity(*klasses,
                         "class A\n  attr_reader :a, 'b', :c\n  def a; end\n  def c; end\nend\n",
                         config)
      source = "class A\n  attr_reader 'b'\n  def b; end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "anonymous class blocks" do
    it "excludes lvasgn values but not parenthesized ones" do
      source = "x = Class.new do\n  def q; end\n  def q; end\nend\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
      expect_lint_parity(*klasses,
                         "x = (Class.new do\n  def q; end\n  def q; end\nend)\n",
                         config)
    end

    it "does not exclude or-assigned or ivar-assigned blocks" do
      expect_lint_parity(*klasses,
                         "Foo ||= Class.new do\n  def x; end\n  def x; end\nend\n",
                         config)
      expect_lint_parity(*klasses,
                         "@x = Class.new do\n  def x; end\n  def x; end\nend\n",
                         config)
    end

    it "keys casgn Class.new WITH args through the anonymous path" do
      expect_lint_parity(*klasses,
                         "Foo = Class.new(Base) do\n  def x; end\n  def x; end\nend\n",
                         config)
    end

    it "collides bare top-level Class.new blocks by key" do
      expect_lint_parity(*klasses,
                         "Class.new do\n  def x; end\nend\nModule.new do\n  def x; end\nend\n",
                         config)
    end

    it "puts defs from the argument list inside the anonymous class" do
      expect_lint_parity(*klasses,
                         "Class.new(def a; end) do\n  def a; end\nend\n",
                         config)
    end

    it "distinguishes single- from multi-statement def bodies (begin wrap)" do
      # Multi-statement bodies lose the path:line scope id, so the twin
      # anonymous defs collide; single-statement bodies keep distinct ids.
      expect_lint_parity(
        *klasses,
        "def m\n  x = 1\n  Class.new { def q; end }\nend\ndef m\n  y = 2\n  Class.new { def q; end }\nend\n",
        config
      )
      single = "def m\n  Class.new { def q; end }\nend\ndef n\n  Class.new { def q; end }\nend\n"
      expect_lint_parity(*klasses, single, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, single, config)).to be_empty
    end

    it "keys sclass-self inside Class.new as Object.x" do
      expect_lint_parity(*klasses,
                         "Class.new do\n  class << self\n    def x; end\n    def x; end\n  end\nend\n",
                         config)
    end
  end

  describe "rescue and ensure scopes" do
    it "allows one redefinition per rescue scope, then flags" do
      expect_lint_parity(
        *klasses,
        "def foo; end\nbegin\n  x\nrescue\n  def foo; end\nend\nbegin\n  y\nrescue\n  def foo; end\nend\n",
        config
      )
    end

    it "treats an ensure-only begin body as the ensure scope" do
      expect_lint_parity(
        *klasses,
        "def foo; end\nbegin\n  def foo; end\nensure\n  def foo; end\nend\ndef foo; end\n",
        config
      )
    end

    it "treats a modifier rescue as the rescue scope" do
      expect_lint_parity(
        *klasses,
        "def foo; end rescue nil\ndef foo; end rescue nil\ndef foo; end\n",
        config
      )
    end
  end

  describe "delegate quirks" do
    let(:as_config) do
      RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new(
          { "AllCops" => { "ActiveSupportExtensionsEnabled" => true } }, "(test)"
        ),
        "(test)"
      )
    end

    it "ignores delegate with a block-pass argument" do
      source = "class A\n  delegate :foo, to: :bar, &b\n  def foo; end\nend\n"
      expect_lint_parity(*klasses, source, as_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, as_config)).to be_empty
    end

    it "prefixes with the to: value for prefix: true string targets" do
      expect_lint_parity(*klasses,
                         "class A\n  delegate :foo, to: 'bar', prefix: true\n  def bar_foo; end\nend\n",
                         as_config)
    end
  end
end
