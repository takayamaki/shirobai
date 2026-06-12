# frozen_string_literal: true

require_relative "empty_lines_around_body_shared"

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLinesAroundModuleBody`
      # (the four-style subset). See `EmptyLinesAroundBodyShared`.
      class EmptyLinesAroundModuleBody < RuboCop::Cop::Base
        include EmptyLinesAroundBodyShared
        extend RuboCop::Cop::AutoCorrector

        SLOT = :empty_lines_around_module_body

        def self.cop_name = "Layout/EmptyLinesAroundModuleBody"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[enforced_style]`.
        def self.bundle_args(config)
          [EmptyLinesAroundBodyShared.style_num(config, badge)]
        end
      end
    end
  end
end
