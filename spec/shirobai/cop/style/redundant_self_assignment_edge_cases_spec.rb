# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/RedundantSelfAssignment`.
#
# The vendor spec exercises the four variable-write flavors (lvasgn / ivasgn /
# cvasgn / gvasgn) plus the canonical setter shape (`other.foo = other.foo.concat(ary)`).
# Real-machine stock probing turned up several quirks the vendor spec does NOT
# cover; pinned here as differential tests so a refactor (or a stricter corpus)
# cannot silently regress them.
#
# - **Setter pattern matches `(call …)`, NOT `(any_block …)`**: a block-wrapped
#   rhs (`obj.foo = obj.foo.concat(ary) { |x| x }`) parses as `(block (send …) …)`
#   in parser-gem, falling out of stock's matcher. The lvasgn arm DOES allow a
#   block on the rhs (`foo = foo.delete_if { true }` is in the vendor spec) —
#   the asymmetry is a real stock quirk, not a bug. In prism the BlockNode is
#   attached as `CallNode.block`, so the setter arm has to skip when the rhs
#   CallNode has a block; the lvasgn arm does not.
# - **`self.foo = self.foo.concat(ary)` IS flagged**: counterintuitive given
#   the vendor spec's "does not register an offense when assigning to attribute
#   of `self`" entry — but that entry's source is `self.foo = foo.concat(ary)`,
#   where the rhs receiver is a *bare* `foo` (lvar or no-receiver call) which
#   cannot structurally equal the outer `self` receiver. When the rhs receiver
#   IS itself `self.foo`, the pattern's `%1` (outer receiver) and the rhs inner
#   receiver are both `(self)` and the pattern matches.
# - **Receiver chain depth >1 works**: `obj.a.foo = obj.a.foo.concat(ary)` is
#   flagged. The pattern compares the full chain `(call %1 …)` against the rhs
#   inner receiver, so the comparison is structural (source-byte equality is a
#   faithful proxy for the contexts we see here).
# - **Receiver chain mismatch at the bottom**: `x.a.foo = y.a.foo.concat(ary)`
#   is NOT flagged — chain receivers differ at the bottom (`x` vs `y`), so the
#   pattern's `%1` binding fails.
# - **Method not in `METHODS_RETURNING_SELF` is rejected**: `foo = foo.upcase`
#   (not destructive) and `obj.foo = obj.foo.upcase!(ary)` (`upcase!` not in the
#   stock set despite the `!`) both stay silent.
# - **`or_asgn` / `op_asgn` are not handled**: `FOO ||= FOO.concat(ary)` and
#   `foo += foo.concat(ary)` get no offense — stock's `on_lvasgn` aliases cover
#   ivasgn/cvasgn/gvasgn but NOT the `*_asgn` variants.
# - **`casgn` is not handled**: `FOO = FOO.concat(ary)` gets no offense — the
#   `ASSIGNMENT_TYPE_TO_RECEIVER_TYPE` map has no `:casgn` entry.
# - **No-receiver rhs is rejected**: `foo = concat(ary)` — `rhs.receiver` is nil,
#   so the variable-assignment arm bails.
# - **Setter exempts the "1-argument" requirement**: `obj.foo, obj.bar = …`
#   isn't a setter call at all (it's a `masgn`), and `obj.[]=(k, v)` lives in
#   a separate `[]=` branch that the cop never enters (only `assignment_method?`
#   names ending in `=` other than `[]=` reach the matcher).
RSpec.describe Shirobai::Cop::Style::RedundantSelfAssignment do
  include EdgeCaseParity

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/RedundantSelfAssignment" => {} },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::RedundantSelfAssignment,
    Shirobai::Cop::Style::RedundantSelfAssignment
  ]

  describe "setter pattern: block on the rhs" do
    it "does NOT flag a setter whose rhs has a block (`obj.foo = obj.foo.concat(ary) { … }`)" do
      src = "other.foo = other.foo.concat(ary) { |x| x }\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "DOES flag a variable assignment whose rhs has a block (`foo = foo.delete_if { … }`)" do
      # Counterpart of the above — stock's `on_lvasgn` matches `(any_block,
      # :call)` so the lvasgn arm allows blocks. Pin both sides explicitly so
      # the asymmetry is visible in the test list.
      src = "foo = foo.delete_if { true }\n"
      expect_lint_parity(*klasses, src, config)
    end
  end

  describe "self setter with self rhs receiver" do
    it "DOES flag `self.foo = self.foo.concat(ary)` (both receivers are `self`)" do
      src = "self.foo = self.foo.concat(ary)\n"
      expect_lint_parity(*klasses, src, config)
    end

    it "does NOT flag `self.foo = foo.concat(ary)` (rhs receiver is bare)" do
      src = "self.foo = foo.concat(ary)\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end
  end

  describe "chained receivers" do
    it "DOES flag `obj.a.foo = obj.a.foo.concat(ary)` (deep chain match)" do
      src = "obj.a.foo = obj.a.foo.concat(ary)\n"
      expect_lint_parity(*klasses, src, config)
    end

    it "does NOT flag `x.a.foo = y.a.foo.concat(ary)` (chain bottom differs)" do
      src = "x.a.foo = y.a.foo.concat(ary)\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end
  end

  describe "rejected method shapes" do
    it "does NOT flag `foo = foo.upcase` (not in METHODS_RETURNING_SELF)" do
      expect_lint_parity(*klasses, "foo = foo.upcase\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo = foo.upcase\n", config)).to be_empty
    end

    it "does NOT flag `obj.foo = obj.foo.upcase!(ary)` (`upcase!` absent from the set)" do
      src = "other.foo = other.foo.upcase!(ary)\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "does NOT flag `foo = concat(ary)` (no rhs receiver)" do
      expect_lint_parity(*klasses, "foo = concat(ary)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo = concat(ary)\n", config)).to be_empty
    end
  end

  describe "non-handled assignment shapes" do
    it "does NOT flag `FOO ||= FOO.concat(ary)` (or-asgn not aliased)" do
      expect_lint_parity(*klasses, "FOO ||= FOO.concat(ary)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "FOO ||= FOO.concat(ary)\n", config)).to be_empty
    end

    it "does NOT flag `foo += foo.concat(ary)` (op-asgn not aliased)" do
      expect_lint_parity(*klasses, "foo += foo.concat(ary)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo += foo.concat(ary)\n", config)).to be_empty
    end

    it "does NOT flag `FOO = FOO.concat(ary)` (casgn not in the type map)" do
      expect_lint_parity(*klasses, "FOO = FOO.concat(ary)\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "FOO = FOO.concat(ary)\n", config)).to be_empty
    end
  end

  describe "autocorrect parity" do
    it "rewrites `foo = foo.concat(ary)` to `foo.concat(ary)` (variable arm)" do
      expect_autocorrect_parity(*klasses, "foo = foo.concat(ary)\n", config)
    end

    it "rewrites `foo = foo.delete_if { true }` keeping the block (variable arm)" do
      expect_autocorrect_parity(*klasses, "foo = foo.delete_if { true }\n", config)
    end

    it "rewrites `other.foo = other.foo.concat(ary)` by dropping the setter prefix" do
      expect_autocorrect_parity(*klasses, "other.foo = other.foo.concat(ary)\n", config)
    end

    it "rewrites `other&.foo = other&.foo&.concat(ary)` keeping the safe-nav chain" do
      expect_autocorrect_parity(*klasses, "other&.foo = other&.foo&.concat(ary)\n", config)
    end

    it "rewrites `self.foo = self.foo.concat(ary)` by dropping the setter prefix" do
      expect_autocorrect_parity(*klasses, "self.foo = self.foo.concat(ary)\n", config)
    end
  end
end
