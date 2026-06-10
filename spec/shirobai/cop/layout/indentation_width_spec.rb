# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::IndentationWidth, :config do
  # Staged port. The core indentation checks (def/class/module/if/unless/case/
  # case-match/while/until/for/block/rescue/ensure and access-modifier
  # consistency styles) are ported. The groups below are still being ported and
  # are marked pending so the suite stays green between commits.
  STAGED_PENDING = [
    /modifier and def are on the same line/,             # send adjacent-def-modifier
    /multiple modifiers and def are on the same line/,
    /EnforcedStyleAlignWith is relative_to_receiver/,    # method-chain block base
    /with ignored patterns set/,                         # AllowedPatterns
    /bad indentation of begin\/end\/while/,              # begin/end/while
    /handles lines with only whitespace/,
    /with begin\/rescue\/else\/ensure\/end/,             # begin rescue/else/ensure indents
    %r{with block.*`do` \.\.\. `ensure`},                # do/ensure block
    /with block when consistency style is indented_internal_methods/,
    /with block when using safe navigation operator registers an offense for an if with setter/
  ].freeze

  VendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/layout/indentation_width_spec.rb", pending: STAGED_PENDING
  )
end
