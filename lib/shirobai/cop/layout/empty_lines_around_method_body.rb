# frozen_string_literal: true

require_relative "empty_lines_around_body_shared"

module Shirobai
  module Cop
    module Layout
      # Drop-in Rust reimplementation of `Layout/EmptyLinesAroundMethodBody`
      # (fixed `no_empty_lines` style, plus the endless-method whole-line
      # offense). See `EmptyLinesAroundBodyShared`.
      class EmptyLinesAroundMethodBody < RuboCop::Cop::Base
        include EmptyLinesAroundBodyShared
        extend RuboCop::Cop::AutoCorrector

        SLOT = :empty_lines_around_method_body

        def self.cop_name = "Layout/EmptyLinesAroundMethodBody"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)
      end
    end
  end
end
