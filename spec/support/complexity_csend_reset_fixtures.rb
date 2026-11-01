# frozen_string_literal: true

# Fixtures for the repeated `&.` discount reset cases shared by
# `spec/shirobai/cop/metrics/{cyclomatic,perceived}_complexity_edge_cases_spec.rb`.
# The header of the cyclomatic spec explains the quirk.
module ComplexityCsendResetFixtures
  # Each body runs between `x&.a` and `x&.b` on a real local `x`.
  RESETS = {
    "a multiple assignment" => "x, _ = foo",
    "an operator assignment" => "x += 1",
    "an `||=` assignment" => "x ||= 1",
    "an `&&=` assignment" => "x &&= 1",
    "a `for` loop variable" => "for x in y; end",
    "a splat in a multiple assignment" => "*x = foo",
    "a nested multiple assignment" => "a, (b, *x) = foo",
    "a `for` loop with several variables" => "for w, x in y; end",
    "a multiple assignment in a `case`/`in` body" => "case z\n  in [*x] then a, x = 1\n  end",
    "a multiple assignment in a pattern guard" => "case z\n  in [*w] if (x, _ = foo)\n    1\n  end"
  }.freeze

  KEEPS = {
    "an array pattern capture" => "case z\n  in [x] then 1\n  end",
    "a binding pattern capture" => "case z\n  in Integer => x then 1\n  end",
    "a hash pattern capture" => "case z\n  in {x:} then 1\n  end",
    "a named regexp capture" => "/(?<x>.)/ =~ s"
  }.freeze

  def self.source(body)
    <<~RUBY
      def m
        x = nil
        x&.a
        #{body.gsub("\n", "\n  ")}
        x&.b
      end
    RUBY
  end

  # `rescue => x` sits in the method body itself, so it gets its own shape.
  RESCUE = <<~RUBY
    def m
      x = nil
      x&.a
    rescue => x
      x&.b
    end
  RUBY

  def self.config(cop_name)
    RuboCop::Config.new(
      cop_name => {
        "Max" => 0,
        "Enabled" => true,
        "AllowedMethods" => [],
        "AllowedPatterns" => [],
        "Exclude" => []
      }
    )
  end
end
