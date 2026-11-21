# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for the 1.92.3 `require_parentheses?`
# (rubocop#16406). An operator send without a dot is wrapped in parentheses
# unless its parser parent allows a command call there: the sole argument of
# a call, a condition, or a statement position (through assignments).
# The Rust side restates the parser parent over prism nodes, so the shapes
# below pin the places where the two trees differ: concretely typed bodies,
# `for` bodies, `when` conditions, `in` guards, rescue modifiers, setters,
# operator assignments and the trailing-comma lookahead.
RSpec.describe Shirobai::Cop::Lint::SafeNavigationChain do
  include EdgeCaseParity

  klasses = [RuboCop::Cop::Lint::SafeNavigationChain, Shirobai::Cop::Lint::SafeNavigationChain]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  {
    "a one-statement for body" => "for i in x\n  a&.b - 1\nend\n",
    "a two-statement for body" => "for i in x\n  foo\n  a&.b - 1\nend\n",
    "a when condition" => "case x\nwhen a&.b - 1\n  foo\nend\n",
    "a when condition of a case without a subject" => "case\nwhen a&.b - 1\n  foo\nend\n",
    "an in guard" => "case x\nin Integer if a&.b - 1\n  foo\nend\n",
    "an in body" => "case x\nin Integer\n  a&.b - 1\nend\n",
    "a case_match subject" => "case a&.b - 1\nin Integer\n  foo\nend\n",
    "a default argument" => "def f(x = a&.b - 1)\nend\n",
    "a multiline parenthesized super" => "def f\n  super(\n    a&.b - 1\n  )\nend\n",
    "super with a block argument" => "def f\n  super a&.b - 1, &blk\nend\n",
    "a multiline parenthesized yield" => "def f\n  yield(\n    a&.b - 1\n  )\nend\n",
    "return with two values" => "def f\n  return a&.b - 1, 2\nend\n",
    "break and next" => "loop { break a&.b - 1 }\nx.each { next a&.b - 1 }\n",
    "an attribute setter" => "x.y = a&.b - 1\n",
    "an explicit setter call" => "x.y=(a&.b - 1)\n",
    "an index setter value" => "x[1] = a&.b - 1\n",
    "an index setter index" => "x[a&.b - 1] = 1\n",
    "an index operator assignment" => "x[a&.b - 1] += 1\n",
    "operator and logical assignments" => "y += a&.b - 1\ny ||= a&.b - 1\n$y &&= a&.b - 1\n",
    "call and index or-assignments" => "x.y ||= a&.b - 1\nx[1] ||= a&.b - 1\n",
    "a multiple assignment" => "y, z = a&.b - 1\n",
    "a chained assignment" => "y = z = a&.b - 1\n",
    "an assignment as an argument" => "foo(y = a&.b - 1)\n",
    "an assignment as a multiline argument" => "foo(\n  y = a&.b - 1\n)\n",
    "an assignment as a condition" => "if y = a&.b - 1\n  foo\nend\nfoo if y = a&.b - 1\n",
    "constant assignments" => "Y = a&.b - 1\nA::Y = a&.b - 1\nA::Y ||= a&.b - 1\n",
    "a rescue exception list" => "begin\nrescue a&.b - 1\nend\n",
    "a lambda body and parameter default" => "-> { a&.b - 1 }\n->(y = a&.b - 1) { y }\n",
    "an sclass body and subject" => "class << self\n  a&.b - 1\nend\nclass << a&.b - 1\nend\n",
    "a superclass" => "class Foo < a&.b - 1\nend\n",
    "a ternary condition" => "a&.b - 1 ? c : d\n",
    "keyword and/or operands" => "foo and a&.b - 1\na&.b - 1 or foo\n",
    "a not operand" => "not a&.b - 1\n",
    "a call with a brace block" => "foo(a&.b - 1) { }\n",
    "a command call with a do block" => "foo a&.b - 1 do\nend\n",
    "a comment after the sole argument" => "foo(a&.b - 1 # x\n)\n",
    "an interpolation" => "\"\#{a&.b - 1}\"\n",
    "while modifiers" => "foo while a&.b - 1\na&.b - 1 while foo\nbegin\n  a&.b - 1\nend while foo\n",
    "an until condition and body" => "until a&.b - 1\n  a&.b - 1\nend\n",
    "a rescue modifier expression" => "a&.b - 1 rescue nil\nfoo(a&.b - 1) rescue nil\n",
    "defined?" => "defined?(a&.b - 1)\n",
    "a keyword argument" => "foo(k: a&.b - 1)\n",
    "a call through .()" => "foo.(a&.b - 1)\n",
    "a multiline receiver chain" => "x.foo\n  .bar(a&.b - 1)\n",
    "if, elsif and else branches" =>
      "if foo\n  a&.b - 1\nelsif a&.b - 1\n  a&.b - 1\nelse\n  a&.b - 1\nend\n",
    "ternary branches" => "foo ? a&.b - 1 : a&.b - 1\n",
    "rescue, else and ensure bodies" =>
      "begin\n  foo\nrescue => e\n  a&.b - 1\nelse\n  a&.b - 1\nensure\n  a&.b - 1\nend\n",
    "an implicit rescue in a def" => "def f\n  foo\nrescue\n  a&.b - 1\nend\n",
    "a two-statement block body" => "[1].each do |x|\n  foo\n  a&.b - 1\nend\n",
    "an assigned if" => "y = if a&.b - 1 then 1 end\n",
    "an assigned parenthesized sequence" => "y = (foo; a&.b - 1)\n",
    "a parenthesized while assignment" => "while (y = a&.b - 1)\nend\n",
    "a comparison as a command argument" => "puts a&.b > 1 && c\n"
  }.each do |label, source|
    it "matches stock offenses and autocorrect for #{label}" do
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end

  # An endless def needs Ruby 3.0, above the helpers' default target.
  it "matches stock for an endless def body under Ruby 3.4" do
    source = "def f = a&.b - 1\n"
    results = klasses.map do |klass|
      cop = klass.new(config, autocorrect: true)
      processed = RuboCop::ProcessedSource.new(source, 3.4)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
      corrector = RuboCop::Cop::Corrector.new(processed)
      report.offenses.each { |o| corrector.merge!(o.corrector) if o.corrector }
      [report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos] }, corrector.rewrite]
    end
    expect(results.first.first).not_to be_empty
    expect(results.last).to eq(results.first)
  end
end
