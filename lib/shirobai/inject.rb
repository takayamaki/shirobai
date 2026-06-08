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
