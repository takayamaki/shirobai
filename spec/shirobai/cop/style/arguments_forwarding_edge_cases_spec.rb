# frozen_string_literal: true

require "spec_helper"

# Edge-case regression specs for `Style/ArgumentsForwarding` that the vendor
# spec does not exercise:
#
# - CRLF sources: `buffer.source != raw_source`, so the wrapper's
#   `bundle_eligible?` gate must send the cop down the standalone entry point
#   (which scans `buffer.source`) so every offset lines up with the parser-gem
#   character index.
# - The paren-less def / call whose kwrest AND block both anonymize with no
#   rest present: stock calls `add_parentheses` TWICE (two different-range
#   offenses), and its corrector stacks the `insert_after(')')` while merging
#   the identical `replace('(')`, producing a DOUBLE `)` (and, on the call, a
#   double `(`). shirobai must reproduce that byte for byte.
# - The 3.3 anonymous-forwarding-in-block guard: forwarding inside a block is
#   NOT correctable below 3.4, but IS at 3.4.
# - `Naming/BlockForwarding EnforcedStyle: explicit` suppresses the block
#   anonymization (the cross-cop config read).
# - Fully anonymous forwarding with no block (`(*, **)`) registers no offense.
#
# Differential style: stock (fresh per file/iteration) and shirobai run over
# the same source at the same target version; offenses and the fully
# autocorrected source must match. Non-vacuous cases assert stock fired first.
RSpec.describe "Style/ArgumentsForwarding edge cases" do
  STOCK = RuboCop::Cop::Style::ArgumentsForwarding
  SHIROBAI = Shirobai::Cop::Style::ArgumentsForwarding

  def af_config(target, cop_extra: {}, block_forwarding_style: nil)
    base = RuboCop::ConfigLoader.default_configuration
    cop_cfg = base["Style/ArgumentsForwarding"].merge(cop_extra).merge("Enabled" => true)
    hash = {
      "AllCops" => base["AllCops"].merge("TargetRubyVersion" => target),
      "Style/ArgumentsForwarding" => cop_cfg
    }
    if block_forwarding_style
      hash["Naming/BlockForwarding"] =
        base["Naming/BlockForwarding"].merge("EnforcedStyle" => block_forwarding_style)
    end
    RuboCop::Config.new(hash, "#{Dir.pwd}/.rubocop.yml")
  end

  # Autocorrect to convergence, parsing at `target` (so 3.2+ anonymous syntax
  # is legal), one cop instance across passes like the vendor harness.
  def af_run(klass, source, config, target)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first = nil
    11.times do |i|
      ps = RuboCop::ProcessedSource.new(src, target)
      ps.config = config
      ps.registry = RuboCop::Cop::Registry.global
      report = RuboCop::Cop::Team.new([cop], config, raise_error: true).investigate(ps)
      offs = report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos, o.message] }.sort
      first ||= offs
      corr = report.correctors.first
      break if corr.nil? || corr.empty?

      rewritten = corr.rewrite
      break if rewritten == src
      raise "did not converge" if i == 10

      src = rewritten
    end
    [first, src]
  end

  def expect_parity(source, target, expect_offenses: true, **cfg)
    config = af_config(target, **cfg)
    stock_offs, stock_src = af_run(STOCK, source, config, target)
    if expect_offenses
      expect(stock_offs).not_to be_empty, "fixture produced no stock offense; fix the source"
    else
      expect(stock_offs).to be_empty, "fixture unexpectedly produced a stock offense"
    end
    sb_offs, sb_src = af_run(SHIROBAI, source, config, target)
    expect(sb_offs).to eq(stock_offs)
    expect(sb_src).to eq(stock_src)
    stock_src
  end

  it "falls back off the bundle path on a CRLF source (byte/char parity)" do
    # `buffer.source` strips the CR, so raw_source != buffer.source and the
    # cop takes the standalone entry point. A forward-all `...` fires at 2.7.
    src = "def foo(*args, **kwargs, &block)\r\n  bar(*args, **kwargs, &block)\r\nend\r\n"
    expect_parity(src, 2.7)
  end

  it "reproduces stock's double `)` when a paren-less def anonymizes kwrest+block" do
    # No rest, so BOTH the kwargs and block offenses call add_parentheses on
    # the paren-less def args: identical `(` merges, `)` inserts stack -> `))`.
    out = expect_parity("def foo **kwargs, &block\n  bar(**kwargs, &block)\nend\n", 3.2)
    expect(out).to eq("def foo(**, &))\n  bar(**, &)\nend\n")
  end

  it "reproduces stock's double `((` / `))` when a paren-less call anonymizes kwrest+block" do
    out = expect_parity("def foo **kwargs, &block\n  bar **kwargs, &block\nend\n", 3.2)
    expect(out).to eq("def foo(**, &))\n  bar((**, &))\nend\n")
  end

  it "does not anonymize forwarding inside a block below 3.4" do
    src = "def foo(*args, &block)\n  do_something do\n    bar(*args, &block)\n  end\nend\n"
    expect_parity(src, 3.2, expect_offenses: false)
  end

  it "anonymizes forwarding inside a block at 3.4" do
    src = "def foo(*args, &block)\n  do_something do\n    bar(*args, &block)\n  end\nend\n"
    expect_parity(src, 3.4)
  end

  it "suppresses block anonymization under Naming/BlockForwarding explicit" do
    # `&block` stays; only the `*` anonymizes.
    out = expect_parity(
      "def foo(*args, &block)\n  bar(*args, &block)\nend\n",
      3.2,
      block_forwarding_style: "explicit"
    )
    expect(out).to eq("def foo(*, &block)\n  bar(*, &block)\nend\n")
  end

  it "registers no offense for fully anonymous non-block forwarding" do
    expect_parity("def foo(*, **)\n  bar(*, **)\nend\n", 3.2, expect_offenses: false)
  end

  # Stock collects `referenced_lvars` from `node.body` ONLY, so a param
  # default expression that reads the rest/kwrest/block name does not block
  # forwarding (probed against stock 1.88.0).
  it "ignores an optarg default expression that reads the restarg name" do
    out = expect_parity("def foo(a = args, *args)\n  bar(*args)\nend\n", 3.2)
    expect(out).to eq("def foo(a = args, *)\n  bar(*)\nend\n")
  end

  it "ignores a kwoptarg default expression that reads the kwrest name" do
    out = expect_parity("def foo(a: kwargs, **kwargs)\n  bar(**kwargs)\nend\n", 3.2)
    expect(out).to eq("def foo(a: kwargs, **)\n  bar(**)\nend\n")
  end

  it "ignores a kwoptarg default expression that reads the block name" do
    out = expect_parity("def foo(a: block, &block)\n  bar(&block)\nend\n", 3.2)
    expect(out).to eq("def foo(a: block, &)\n  bar(&)\nend\n")
  end

  it "still blocks forwarding on a body reference" do
    expect_parity(
      "def foo(*args)\n  x = args\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  # Prism represents each operator-assignment / multiple-assignment / for /
  # rescue target with its own node kind, none of which is a plain
  # `LocalVariable{Read,Write}Node`. Stock reaches every one of them through
  # `each_descendant(:lvar, :lvasgn)` (they are all `:lvasgn` in parser), so a
  # def whose body assigns to the forwardable name that way must NOT forward.
  # This is the Discourse `spec/support/prefabrication.rb` `fab!` shape (probed
  # against stock 1.88.0).
  it "blocks forwarding when the block arg is `||=`-assigned (or-write)" do
    expect_parity(
      "def foo(&blk)\n  blk ||= proc {}\n  bar(&blk)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  it "blocks forwarding when the rest arg is `&&=`-assigned (and-write)" do
    expect_parity(
      "def foo(*args)\n  args &&= []\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  it "blocks forwarding when the rest arg is `+=`-assigned (operator-write)" do
    expect_parity(
      "def foo(*args)\n  args += [1]\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  it "blocks forwarding when the rest arg is a multiple-assignment target" do
    expect_parity(
      "def foo(*args)\n  args, _ = [1, 2]\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  it "blocks forwarding when the rest arg is a `for` loop variable" do
    expect_parity(
      "def foo(*args)\n  for args in [1] do end\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  it "blocks forwarding when the rest arg is a rescue binding" do
    expect_parity(
      "def foo(*args)\n  begin\n    x\n  rescue => args\n    y\n  end\n  bar(*args)\nend\n",
      3.2,
      expect_offenses: false
    )
  end

  # A pattern-match binding shares prism's `LocalVariableTargetNode` kind but is
  # `:match_var` in parser, not `:lvasgn`. Stock does NOT count it as a body
  # reference, so forwarding still fires (probed against stock 1.88.0).
  it "still forwards when the name is only a `case/in` capture binding" do
    expect_parity(
      "def foo(*args)\n  case 1\n  in Integer => args\n    y\n  end\n  bar(*args)\nend\n",
      3.2
    )
  end

  it "still forwards when the name is only a one-line pattern binding" do
    expect_parity(
      "def foo(*args)\n  1 => args\n  bar(*args)\nend\n",
      3.2
    )
  end

  it "still forwards when the name is only an array-pattern binding" do
    expect_parity(
      "def foo(*args)\n  case [1]\n  in [args]\n    y\n  end\n  bar(*args)\nend\n",
      3.2
    )
  end
end
