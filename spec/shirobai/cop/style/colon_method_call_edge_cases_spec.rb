# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/ColonMethodCall`.
#
# The vendor spec covers the canonical detection / autocorrect path and the
# main exclusions (camel-case constructor, Java static type, Java package
# namespace), but a number of structural quirks were uncovered during
# implementation by probing stock rubocop directly. Those quirks are pinned
# here because corpus parity is disposable (clean corpora produce few
# offenses for this cop, so a regression might not surface) and because the
# AST shape the cop relies on differs subtly between parser-gem and prism in
# a few corner cases:
#
# - **`(const nil? :Java)` vs `cbase` receiver**: stock's `java_type_node?`
#   pattern is `(send (const nil? :Java) _)`. The pattern's `nil?` matches a
#   **nil receiver on the `const` node**, NOT a `cbase` receiver. So
#   `::Java::int` (where the receiver of `int` is `(const cbase :Java)`) is
#   FLAGGED by stock — the java-type guard does not apply when `Java` is
#   resolved at the top level via `::`. In prism a top-level `::Java`
#   constant is a `ConstantPathNode` (not a `ConstantReadNode`), so the
#   wrapper's `receiver.as_constant_read_node()` gate is structurally
#   correct: only the bare `ConstantReadNode` named `Java` triggers the
#   exclusion. Vendor spec has no example for the `cbase` variant, but the
#   distinction is load-bearing (a refactor of the guard could silently
#   regress).
# - **Java interop chains (`Java::foo::bar`)**: stock's `java_interop?`
#   guard (1.88.0+) walks the receiver chain to its root and checks
#   `java_root?` (`(const nil? :Java)`). The entire chain rooted at a bare
#   `Java` constant is excluded. The `cbase` form (`::Java::foo::bar`) is
#   NOT excluded because the root is a `ConstantPathNode`, not a bare
#   `ConstantReadNode`. Pinned because a future refactor could silently
#   regress the chain-walking or cbase distinction.
# - **Non-constant receiver shapes**: vendor spec only exercises `Class`
#   (`ConstantReadNode`) and `test` (`CallNode`) as receivers. In stock the
#   only structural guards depend on the message name (camel case) and the
#   receiver shape (java type node). Other receiver shapes — instance
#   variable (`@x::y`), expression in parens (`(1+2)::to_s`), explicit
#   self (`self::y`) — must all flag. Pinned because the wrapper relies on
#   prism's `receiver().is_some()` gate which covers every receiver kind
#   indistinguishably.
# - **`csend` (safe navigation)**: `foo&.bar` uses `&.` as its operator, not
#   `::`. csend with `::` is not valid Ruby. The wrapper checks the operator
#   bytes are `::` exactly, so safe-navigation calls fall through. Pinned to
#   guard the operator-bytes check against accidental relaxation to "any
#   two-byte operator".
# - **Constant-only path (`Foo::Bar::Baz`)**: this is pure constant
#   resolution (`ConstantPathNode`), not a method call — there is no
#   `CallNode` and stock never visits it via `on_send`. Pinned because a
#   future broadening of the visitor (e.g. visiting `ConstantPathNode` to
#   share work with another cop) could accidentally synthesise offenses
#   here.
# - **Camel-case method on non-Java receiver (`Foo::Bar()` vs `Foo::baz`)**:
#   the `camel_case_method?` guard checks the method name's FIRST byte for
#   `[A-Z]`, not the receiver. Pinned because a future refactor mis-reading
#   `camel_case_method?` as "receiver is camel-case" would let a normal
#   constructor through.
# - **Visitor recursion**: vendor spec only places offending calls at the
#   top level. Pinned that the visitor recurses into method bodies, block
#   bodies, and into the *argument expressions* of an enclosing call, so a
#   future walk-pruning refactor cannot silently regress nested cases.
RSpec.describe Shirobai::Cop::Style::ColonMethodCall do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Style::ColonMethodCall,
    Shirobai::Cop::Style::ColonMethodCall
  ]

  describe "java-type-node receiver gating" do
    it "does NOT flag `Java::int` (top-level Java const receiver)" do
      expect_lint_parity(*klasses, "Java::int\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Java::int\n", config)).to be_empty
    end

    it "DOES flag `::Java::int` (cbase Java; pattern `nil?` rejects cbase)" do
      # Stock pattern `(const nil? :Java)` matches a nil receiver on the const
      # node only. `(const cbase :Java)` is NOT nil, so the java-type guard
      # does not apply and stock flags this call (verified by probe).
      expect_lint_parity(*klasses, "::Java::int\n", config)
    end

    it "does NOT flag `Java::foo::bar` (entire chain rooted at Java is excluded)" do
      # The `java_interop?` guard walks the receiver chain to its root;
      # since the root is `Java`, the entire chain is excluded.
      expect_lint_parity(*klasses, "Java::foo::bar\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Java::foo::bar\n", config)).to be_empty
    end

    it "does NOT flag `Java::com.foo` (java type then `.foo`)" do
      # Inner `Java::com` is java-typed (excluded). Outer `.foo` uses `.` not
      # `::`, so nothing flagged.
      expect_lint_parity(*klasses, "Java::com.foo\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Java::com.foo\n", config)).to be_empty
    end
  end

  describe "receiver shape coverage" do
    it "flags `@ivar::y` (instance variable receiver)" do
      expect_autocorrect_parity(*klasses, "@x::y\n", config)
    end

    it "flags `(1 + 2)::to_s` (parenthesised expression receiver)" do
      expect_autocorrect_parity(*klasses, "(1 + 2)::to_s\n", config)
    end

    it "flags `self::y` (explicit self receiver)" do
      expect_autocorrect_parity(*klasses, "self::y\n", config)
    end
  end

  describe "operator-bytes gating" do
    it "does NOT flag `foo&.bar` (safe navigation, `&.` is two bytes but not `::`)" do
      # Pinned to guard the operator-bytes check against accidental
      # relaxation to "any two-byte operator".
      expect_lint_parity(*klasses, "foo&.bar\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo&.bar\n", config)).to be_empty
    end

    it "does NOT flag `Foo.bar` (`.` connector is single-byte)" do
      expect_lint_parity(*klasses, "Foo.bar\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Foo.bar\n", config)).to be_empty
    end
  end

  describe "constant-only path is not a send" do
    it "does NOT flag `Foo::Bar::Baz` (pure constant resolution)" do
      # `ConstantPathNode` chain, no CallNode. Pinned so a future visitor
      # broadening cannot synthesise offenses here.
      expect_lint_parity(*klasses, "Foo::Bar::Baz\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Foo::Bar::Baz\n", config)).to be_empty
    end

    it "does NOT flag `Foo::Bar::BAZ_CONST` (lowercase tail still constant)" do
      # The tail `BAZ_CONST` starts with uppercase so it parses as a constant
      # path, not a method call. Pinned alongside the all-camel variant.
      expect_lint_parity(*klasses, "Foo::Bar::BAZ_CONST\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Foo::Bar::BAZ_CONST\n", config)).to be_empty
    end
  end

  describe "camel-case method name gating" do
    it "does NOT flag `Tip::Top(arg)` (camel-case method name)" do
      # Stock's `camel_case_method?` checks the *method name* first byte for
      # `[A-Z]`. Vendor spec covers this.
      expect_lint_parity(*klasses, "Tip::Top(some_arg)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Tip::Top(some_arg)\n", config)).to be_empty
    end

    it "flags `Foo::bar(arg)` (lowercase method on camel-case receiver)" do
      # Camel-case is about the *method name*, not the receiver. Pinned so a
      # future refactor reading `camel_case_method?` as "receiver is
      # camel-case" cannot let the canonical-style call through.
      expect_autocorrect_parity(*klasses, "Foo::bar(arg)\n", config)
    end
  end

  describe "visitor recursion" do
    it "flags a `::` call buried inside a `def` body" do
      expect_lint_parity(*klasses, "def m\n  test::method_name\nend\n", config)
    end

    it "flags a `::` call buried inside a block body" do
      expect_lint_parity(*klasses, "list.each do\n  test::method_name\nend\n", config)
    end

    it "flags a `::` call appearing as the argument of an enclosing call" do
      # The outer `puts(...)` is itself unflagged (`.` style), but its
      # argument is a `Class::method` call which IS flagged. Pinned that the
      # visitor descends into argument expressions.
      expect_lint_parity(*klasses, "puts(Class::method_name(arg))\n", config)
    end
  end

  describe "multi-byte source" do
    it "flags `あ::method_name(arg)` with byte-correct offense range" do
      # The receiver is a non-ASCII bare identifier parsed as a no-arg call
      # `あ` (`send(nil, :あ)`). The `::` token sits after the multibyte
      # receiver, so the wrapper's SourceOffsets byte→char conversion is on
      # the offense path; both the offense and autocorrect ranges must come
      # out at the matching character offsets stock produces.
      expect_autocorrect_parity(*klasses, "あ::method_name(arg)\n", config)
    end
  end

  describe "autocorrect output" do
    it "replaces `::` with `.` byte-for-byte (single offense)" do
      # The vendor spec's `expect_correction` already covers the corrected
      # source byte-for-byte, but pinned here as a differential against
      # stock so a future refactor that, e.g., expands the replace range to
      # include surrounding spaces would catch it.
      expect_autocorrect_parity(*klasses, "test::method_name\n", config)
    end

    it "replaces `::` with `.` in `Class::method_name(arg, arg2)`" do
      expect_autocorrect_parity(*klasses, "Class::method_name(arg, arg2)\n", config)
    end
  end
end
