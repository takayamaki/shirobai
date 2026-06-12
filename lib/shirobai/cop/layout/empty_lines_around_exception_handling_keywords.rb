# frozen_string_literal: true

require_relative "empty_lines_around_body_shared"

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of
      # `Layout/EmptyLinesAroundExceptionHandlingKeywords` (blank lines
      # directly above/below `rescue` / `else` / `ensure` in def, block and
      # `begin` bodies; stock's missing `on_itblock` alias is mirrored).
      # See `EmptyLinesAroundBodyShared`.
      class EmptyLinesAroundExceptionHandlingKeywords < RuboCop::Cop::Base
        include EmptyLinesAroundBodyShared
        extend RuboCop::Cop::AutoCorrector

        SLOT = :empty_lines_around_exception_handling_keywords

        def self.cop_name = "Layout/EmptyLinesAroundExceptionHandlingKeywords"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)
      end
    end
  end
end
