# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: prism write-target bypass.
#
# Prism dispatches a write-target's same-shaped read node through a
# CONCRETELY-TYPED field, bypassing the generic visitor: const-path writes
# (`A::B = 1`, `A::B::C = 1`, op-asgn / and-asgn / or-asgn variants) carry no
# read `ConstantPathNode` on the generic walk, and attribute writes
# (`self.foo ||= 1`, `foo.bar = 1`, `Foo::Bar.baz = 1`) split into call-write
# node kinds whose receiver/CallTargetNode is a read node the generic visitor
# never re-enters. SpaceAroundMethodCallOperator only flags the `.`/`::` of a
# READ method call; if the wrapper fails to skip these write targets it would
# false-positive, and if it over-broadly suppressed reads it would miss them.
#
# The vendor + non_ascii fixtures only walk READ paths (`あ.foo . bar`,
# `RuboCop:: Cop`), so they never touch a write target. These cases pin that
# stock and shirobai BOTH stay at zero offenses over write targets (no false
# positive), and that the read-context positive control still diverges from
# zero identically.
RSpec.describe Shirobai::Cop::Layout::SpaceAroundMethodCallOperator do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::SpaceAroundMethodCallOperator,
    Shirobai::Cop::Layout::SpaceAroundMethodCallOperator
  ]

  # Write targets: stock emits NO offense (the `::` / `.` belongs to a write
  # target, not a read method call). shirobai must agree at zero — a dropped
  # visitor override would false-positive on these.
  {
    "const-path write (`A::B = 1`)" => "A::B = 1\n",
    "nested const-path write (`A::B::C = 1`)" => "A::B::C = 1\n",
    "const-path or-asgn (`A::B ||= 1`)" => "A::B ||= 1\n",
    "attr op-asgn on self (`self.foo ||= 1`)" => "self.foo ||= 1\n",
    "attr and-asgn on ivar receiver (`@a.b &&= 1`)" => "@a.b &&= 1\n",
    "attr write (`foo.bar = 1`)" => "foo.bar = 1\n",
    "const-receiver attr write (`Foo::Bar.baz = 1`)" => "Foo::Bar.baz = 1\n"
  }.each do |label, source|
    it "emits no offense for a write target: #{label}" do
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      # Belt-and-braces: confirm stock really is zero (the assertion above
      # only checks shirobai == stock, which would also hold if both were
      # spuriously non-zero).
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  # Positive control: a genuine READ method call WITH surrounding space still
  # produces offenses, and stock/shirobai must agree on them and on the
  # autocorrected (space-removed) source. Guards against the write-target skip
  # being over-broad and swallowing real reads.
  it "still flags spaces around a read `.`/`::` operator (positive control)" do
    source = "foo . bar\nRuboCop:: Cop\n"
    expect_lint_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end
end
