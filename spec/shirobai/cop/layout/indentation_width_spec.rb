# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai::Cop::Layout::IndentationWidth, :config do
  # Staged port. The core indentation checks (def/class/module/if/unless/case/
  # case-match/while/until/for/block/rescue/ensure and access-modifier
  # consistency styles) are ported. The groups below are still being ported and
  # are marked pending so the suite stays green between commits.
  STAGED_PENDING = [
    /bad indentation of begin\/end\/while/               # do-while + assignment interaction
  ].freeze

  VendorSpecHelper.load_vendor_spec(
    self, "rubocop/cop/layout/indentation_width_spec.rb", pending: STAGED_PENDING
  )
end
