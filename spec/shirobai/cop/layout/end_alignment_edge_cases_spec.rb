# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: elsif shares the outer `if`'s `end`.
#
# `elsif` is a nested `IfNode` whose `loc.end` is nil (the `end` keyword belongs
# to the outermost `if`). If the wrapper does not exclude the "elsif" syntactic
# subtree from end-alignment, the variable/start-of-line styles drive the
# `end`'s autocorrect to OSCILLATE (column 4 <-> column 15) and never converge,
# which is the worst kind of drop-in breakage (an infinite autocorrect loop).
# The default `keyword` style aligns `end` with the outer `if`; this pins that
# the autocorrect CONVERGES (and is stable on a re-run) and that stock and
# shirobai agree on offenses and corrected source.
#
# Corpus-only before this spec (the vendor + non_ascii fixtures use `var = if
# test\nend` with no elsif).
RSpec.describe Shirobai::Cop::Layout::EndAlignment do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::EndAlignment,
    Shirobai::Cop::Layout::EndAlignment
  ]

  # An elsif chain whose `end` is misaligned: the autocorrect must move the
  # `end` to align with the outer `if` and then STOP (not ping-pong between
  # the `if` column and a `elsif`-derived column).
  misaligned = "var = if c\n  a\nelsif d\n  b\n  end\n"
  # An elsif chain already aligned with the outer `if` (no offense).
  aligned = "var = if c\n  a\nelsif d\n  b\n      end\n"

  it "aligns the shared `end` to the outer `if` and converges" do
    expect_lint_parity(*klasses, misaligned, config)
    corrected = expect_autocorrect_parity(*klasses, misaligned, config)
    # The corrected source is itself offence-free, i.e. the autocorrect reached
    # a fixpoint and does not oscillate on a second pass.
    expect(lint_offenses(klasses.first, corrected, config)).to be_empty
    expect(lint_offenses(klasses.last, corrected, config)).to be_empty
    expect(autocorrect_run(klasses.last, corrected, config).last).to eq(corrected)
  end

  it "emits no offense when the shared `end` already aligns with the outer `if`" do
    expect_lint_parity(*klasses, aligned, config, expect_offenses: false)
    expect(lint_offenses(klasses.first, aligned, config)).to be_empty
  end

  # 1.92.2 (rubocop#16142): `on_class` / `on_module` are aliases of
  # `on_sclass`, so a `class` / `module` / `sclass` that is the direct value
  # of an assignment takes `check_asgn_alignment` (variable style aligns with
  # the assignment) instead of `check_other_alignment` (variable style aligned
  # with the keyword, while its autocorrect aligned with the assignment: an
  # infinite loop).
  def style_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/EndAlignment" => { "EnforcedStyleAlignWith" => style } }, "(test)"
      ),
      "(test)"
    )
  end

  scope_sources = {
    "class end at the assignment" => "Foo = class Bar\n  x\nend\n",
    "class end at the keyword" => "Foo = class Bar\n  x\n      end\n",
    "module end at the assignment" => "Foo = module Bar\n  x\nend\n",
    "module end at the keyword" => "Foo = module Bar\n  x\n      end\n",
    "indented class assignment" => "def m\n  @foo = class Bar\n    x\n  end\nend\n",
    "sclass assignment" => "x = class << self\n  y\nend\n",
    "or-assignment" => "Foo ||= module Bar\n  x\n        end\n",
    "multiple assignment" => "a, b = class X\n  y\nend\n",
    "index or-assignment" => "h[:k] ||= class X\n  y\nend\n",
    "line break before the keyword" => "Foo =\n  class Bar\n    x\nend\n",
    "setter call (not an assignment)" => "x.y = class Z\n  w\nend\n",
    "parenthesized value (not a direct child)" => "Foo = (class Bar\n  x\nend)\n",
    "non-ASCII variable" => "変数 = class Bar\n  x\n     end\n",
    # Not a direct child: `check_other_alignment`, but the variable style
    # still aligns the autocorrect with `assignment_or_operator_method`.
    "class with a call chained" => "Foo = class Bar\n  x\n      end.freeze\n",
    "parenthesized module" => "x = (module M\n  y\n    end)\n",
    "operator argument" => "foo + class X\n  y\nend\n",
    "operator receiver" => "class X\n  y\n  end + 1\n",
    "last call argument" => "puts(class X\n  y\nend)\n",
    "argument before a block pass" => "puts(class X\n  y\nend, &b)\n",
    "class in a block" => "foo bar do\n  class B\n    z\n      end\nend\n",
    "shovel" => "list << class X\n  y\nend\n",
    "setter with the end at the keyword" => "a.b = module M\n  y\n      end\n",
    "if before an operator" => "if c\n  1\n  end + 2\n"
  }

  scope_sources.each do |name, source|
    %w[keyword variable start_of_line].each do |style|
      it "matches stock for a #{name} (#{style})" do
        cfg = style_config(style)
        expect_lint_parity(*klasses, source, cfg, expect_offenses: false)
        expect(correction_edits(klasses.last, source, cfg)).to eq(correction_edits(klasses.first, source, cfg))
        expect_autocorrect_parity(*klasses, source, cfg)
      end
    end
  end

  it "accepts an end aligned with the assignment under the variable style" do
    source = scope_sources["class end at the assignment"]
    cfg = style_config("variable")
    expect_lint_parity(*klasses, source, cfg, expect_offenses: false)
    expect(lint_offenses(klasses.first, source, cfg)).to be_empty
  end

  it "moves an end at the keyword to the assignment under the variable style" do
    source = scope_sources["non-ASCII variable"]
    cfg = style_config("variable")
    expect_lint_parity(*klasses, source, cfg)
    expect_correction_edits_parity(*klasses, source, cfg)
    expect(expect_autocorrect_parity(*klasses, source, cfg)).to eq("変数 = class Bar\n  x\nend\n")
  end
end
