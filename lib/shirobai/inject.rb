# frozen_string_literal: true

module Shirobai
  module Inject
    def self.replace_cops!
      registry = RuboCop::Cop::Registry.global

      Shirobai::Cop.constants(false).each do |department|
        mod = Shirobai::Cop.const_get(department)
        next unless mod.is_a?(Module)

        mod.constants(false).each do |cop_name|
          klass = mod.const_get(cop_name)
          next unless klass < RuboCop::Cop::Base

          original = registry.find { |c| c.cop_name == klass.cop_name }
          next unless original

          registry.dismiss(original)
          registry.enlist(klass)
        end
      end
    end
  end
end

require_relative "cop/base"
require_relative "dispatch"
require_relative "cop/lint/debugger"
require_relative "cop/metrics/block_length"
require_relative "cop/metrics/block_nesting"
require_relative "cop/metrics/cyclomatic_complexity"
require_relative "cop/metrics/perceived_complexity"
require_relative "cop/naming/variable_number"
require_relative "cop/lint/safe_navigation_chain"
require_relative "cop/layout/multiline_operation_indentation"
require_relative "cop/layout/multiline_method_call_indentation"
require_relative "cop/layout/dot_position"
