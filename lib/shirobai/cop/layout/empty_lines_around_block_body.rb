# frozen_string_literal: true

require_relative "empty_lines_around_body_shared"

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLinesAroundBlockBody`
      # (`no_empty_lines` / `empty_lines`; call, super and lambda blocks,
      # numbered and `it` parameter forms included). See
      # `EmptyLinesAroundBodyShared`.
      class EmptyLinesAroundBlockBody < RuboCop::Cop::Base
        include EmptyLinesAroundBodyShared
        extend RuboCop::Cop::AutoCorrector

        SLOT = :empty_lines_around_block_body

        def self.cop_name = "Layout/EmptyLinesAroundBlockBody"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Packed args for the bundled run: `[enforced_style]`.
        def self.bundle_args(config)
          [EmptyLinesAroundBodyShared.style_num(config, badge)]
        end
      end
    end
  end
end
