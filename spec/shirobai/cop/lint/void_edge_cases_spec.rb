# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for 1.92 `void_expression_range`
# (rubocop#15796): a void heredoc is removed together with its body, up to
# the end of its terminator line. prism's heredoc closing location includes
# the trailing newline while parser's `heredoc_end` does not, so the Rust
# range is snapped back by one byte. These shapes are not in the vendor spec.
RSpec.describe Shirobai::Cop::Lint::Void do
  include EdgeCaseParity

  klasses = [RuboCop::Cop::Lint::Void, Shirobai::Cop::Lint::Void]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  {
    "an interpolated heredoc" => "<<~A\n  \#{x}\nA\ntop\n",
    "a dash heredoc in a method body" => "def f\n  <<-A\n  x\n  A\n  1\nend\n",
    "two void heredocs in a row" => "<<~A\n  a\nA\n<<~B\n  b\nB\ntop\n",
    "a heredoc with an empty body" => "<<~A\nA\ntop\n"
  }.each do |label, source|
    it "matches stock offenses and autocorrect for #{label}" do
      expect_lint_parity(*klasses, source, config)
      expect_autocorrect_parity(*klasses, source, config)
    end
  end
end
