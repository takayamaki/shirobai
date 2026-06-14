# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/FirstArrayElementIndentation`.
#
# The vendor spec exercises the three `EnforcedStyle`s on hand-written `[\n
# ...\n]` literals, but does NOT pin a Ruby-wrapper bug discovered via the
# Discourse parity diff (HEAD before 2026-06-14 fix):
#
#   The wrapper's `node_for(range)` resolves the AST node whose `source_range`
#   exactly matches the offense range, so `AlignmentCorrector` can use a NODE
#   (skipping interior string-literal lines) for the first-element offense and
#   a RANGE for the `]` offense. Pre-fix it called `node.source_range.begin_pos`
#   without nil-guarding, so parser-AST synthetic nodes (implicit `:begin`
#   wrappers around block bodies, `:mlhs` wrappers, etc.) without a
#   `source_range` raised `NoMethodError`. The cop caught the error, but the
#   raise short-circuited the per-offense `each` loop after the first offense
#   was recorded — so multi-offense files (e.g. `%i[...]` / `%w[...]` arrays
#   in deeply nested `class << self` scopes) silently lost their `]` offense.
#   Real CLI diff on Discourse: 93 missing `]` offenses; Redmine: 1.
#
# Pinned here as differential regressions against the 1.87-pinned stock.
RSpec.describe Shirobai::Cop::Layout::FirstArrayElementIndentation do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::FirstArrayElementIndentation,
    Shirobai::Cop::Layout::FirstArrayElementIndentation
  ]

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  it "reports both the first element AND the right bracket in a `%i[]` literal" do
    # Minimised from `app/controllers/categories_controller.rb` (Discourse).
    # Both first element AND `]` are misindented: stock reports both. Pre-fix
    # shirobai's `node_for` raised on a synthetic node before the second
    # offense was added, so only the first element appeared.
    src = <<~RUBY
      class CategoriesController
        requires_login except: %i[
                         index
                         categories_and_latest
                       ]
      end
    RUBY
    stock = expect_lint_parity(*klasses, src, config)
    expect(stock.size).to eq(2)
    expect(stock.map { |o| o[2] }).to include(
      a_string_matching(/Use 2 spaces for indentation in an array, relative to the start of the line where the left square bracket is\./),
      a_string_matching(/Indent the right bracket the same as the start of the line where the left bracket is\./)
    )
  end

  it "reports both the first element AND `]` for a `%w[]` inside a method call" do
    # Minimised from `lib/plugins/acts_as_attachable/lib/acts_as_attachable.rb`
    # (Redmine). The `]` uses the after-paren base because the array is the
    # `Concurrent::Set.new(...)` argument on the same line as `(`.
    src = <<~RUBY
      class ObjectTypeConstraint
              cattr_accessor :object_types

              self.object_types = Concurrent::Set.new(%w[
                issues versions news messages
              ])

              class << self
                def matches?(request)
                  request.path_parameters[:object_type] =~ /^(.*)$/
                end
              end
            end
    RUBY
    stock = expect_lint_parity(*klasses, src, config)
    expect(stock.size).to eq(2)
    expect(stock.map { |o| o[2] }.uniq).to include(
      a_string_matching(/the first position after the preceding left parenthesis/)
    )
  end

  it "negative control: a correctly indented `%w[]` produces no offense" do
    # Guard against the wrapper raising on synthetic nodes when there is
    # nothing to report (which would also crash investigation).
    src = <<~RUBY
      class Foo
        self.types = Concurrent::Set.new(%w[
                                           a b c
                                         ])
      end
    RUBY
    expect_lint_parity(*klasses, src, config, expect_offenses: false)
  end
end
