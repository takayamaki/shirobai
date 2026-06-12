# frozen_string_literal: true

require_relative "empty_lines_around_body_shared"

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLinesAroundBeginBody`
      # (fixed `no_empty_lines` style over the whole `begin`..`end` extent,
      # rescue/ensure sections included). See `EmptyLinesAroundBodyShared`.
      class EmptyLinesAroundBeginBody < RuboCop::Cop::Base
        include EmptyLinesAroundBodyShared
        extend RuboCop::Cop::AutoCorrector

        SLOT = :empty_lines_around_begin_body

        def self.cop_name = "Layout/EmptyLinesAroundBeginBody"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)
      end
    end
  end
end
