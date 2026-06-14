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

require_relative "source_offsets"
require_relative "cop/base"
require_relative "dispatch"
require_relative "cop/lint/debugger"
require_relative "cop/metrics/block_length"
require_relative "cop/metrics/method_length"
require_relative "cop/metrics/block_nesting"
require_relative "cop/metrics/cyclomatic_complexity"
require_relative "cop/metrics/perceived_complexity"
require_relative "cop/metrics/abc_size"
require_relative "cop/naming/variable_number"
require_relative "cop/naming/method_name"
require_relative "cop/naming/predicate_prefix"
require_relative "cop/lint/safe_navigation_chain"
require_relative "cop/lint/useless_access_modifier"
require_relative "cop/lint/void"
require_relative "cop/layout/argument_alignment"
require_relative "cop/layout/closing_parenthesis_indentation"
require_relative "cop/layout/multiline_operation_indentation"
require_relative "cop/layout/multiline_method_call_indentation"
require_relative "cop/layout/dot_position"
require_relative "cop/layout/empty_line_between_defs"
require_relative "cop/layout/empty_lines_around_arguments"
require_relative "cop/layout/end_alignment"
require_relative "cop/layout/def_end_alignment"
require_relative "cop/layout/block_alignment"
require_relative "cop/layout/else_alignment"
require_relative "cop/layout/empty_lines_around_method_body"
require_relative "cop/layout/empty_lines_around_class_body"
require_relative "cop/layout/empty_lines_around_module_body"
require_relative "cop/layout/empty_lines_around_block_body"
require_relative "cop/layout/empty_lines_around_begin_body"
require_relative "cop/layout/empty_lines_around_exception_handling_keywords"
require_relative "cop/layout/first_argument_indentation"
require_relative "cop/layout/first_array_element_indentation"
require_relative "cop/layout/first_hash_element_indentation"
require_relative "cop/layout/hash_alignment"
require_relative "cop/layout/indentation_consistency"
require_relative "cop/layout/indentation_width"
require_relative "cop/layout/line_length"
require_relative "cop/layout/trailing_empty_lines"
require_relative "cop/layout/space_around_method_call_operator"
require_relative "cop/layout/space_around_keyword"
require_relative "cop/layout/space_inside_block_braces"
require_relative "cop/style/block_delimiters"
require_relative "cop/style/hash_each_methods"
require_relative "cop/style/hash_syntax"
require_relative "cop/style/string_literals"
require_relative "cop/style/string_literals_in_interpolation"
require_relative "cop/style/trailing_comma_in_arguments"
require_relative "cop/style/line_end_concatenation"
require_relative "cop/style/redundant_self"
