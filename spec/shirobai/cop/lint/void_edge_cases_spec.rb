# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for 1.92 `void_expression_range`
# (rubocop#15796): a void heredoc is removed together with its body, up to
# the end of its terminator line. prism's heredoc closing location includes
# the trailing newline while parser's `heredoc_end` does not, so the Rust
# range is snapped back by one byte. These shapes are not in the vendor spec.
#
# 1.92.3 shapes: `operation?` (rubocop#16290) looks for a comma anywhere
# between the only argument and the end of the send, and a literal that
# holds any parser `begin` is not corrected (rubocop#16282). The Rust side
# restates parser `begin` over prism nodes, so the shapes below pin it.
RSpec.describe Shirobai::Cop::Lint::Void do
  include EdgeCaseParity

  klasses = [RuboCop::Cop::Lint::Void, Shirobai::Cop::Lint::Void]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  {
    "an interpolated heredoc" => "<<~A\n  \#{x}\nA\ntop\n",
    "a dash heredoc in a method body" => "def f\n  <<-A\n  x\n  A\n  1\nend\n",
    "two void heredocs in a row" => "<<~A\n  a\nA\n<<~B\n  b\nB\ntop\n",
    "a heredoc with an empty body" => "<<~A\nA\ntop\n",
    "a comma in a comment after the only argument" => "x + y\na.+(b # c, d\n)\nnil\n",
    "an operator call with an argument and a block argument" => "x + y\na.+(b, &c)\nnil\n",
    "a parenthesesless operator call with two arguments" => "x + y\na.+ b, c\nnil\n",
    "a unary operator call with one argument" => "a.!(b)\nnil\n",
    "a unary operator call with two arguments" => "x + y\na.-@(b, c)\nnil\n",
    "an operator call with forwarded arguments" => "def f(...)\n  x + y\n  a.+(...)\n  nil\nend\n",
    "an operator call with a parenthesized argument" => "a.+((b))\nnil\n",
    "a string with an interpolated variable" => "\"\#@foo\"\ntop\n",
    "a string with an empty interpolation" => "\"\#{}\"\ntop\n",
    "adjacent string literals" => "\"a\" \"b\"\ntop\n",
    "a hash with a parenthesized double splat" => "{**(x)}\ntop\n",
    "a hash with a double splat of a two-statement block" => "{**foo { a; b }}\ntop\n",
    "a hash with a double splat of a one-statement block" => "{**foo { a }}\ntop\n",
    "a hash with a double splat of a plain begin" => "{**foo { begin; a; b; end }}\ntop\n",
    "a hash with a double splat of a begin with rescue" =>
      "{**foo { begin; a; b; rescue; end }}\ntop\n",
    "a frozen interpolated string" => "\"\#{x}\".freeze\ntop\n",
    "an interpolated regexp" => "/\#{x}/\ntop\n"
  }.each do |label, source|
    it "matches stock offenses and autocorrect for #{label}" do
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end
end
