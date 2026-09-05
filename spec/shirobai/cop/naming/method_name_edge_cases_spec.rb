# frozen_string_literal: true

require "spec_helper"

# `ForbiddenIdentifiers` travels to Rust (core list 29) so that a config
# setting only that list stays on the bundle's filtered fast path. The
# filter must keep a forbidden name whose STYLE is fine (`__id__` is valid
# snake_case) — dropping it with the other valid sites would lose stock's
# forbidden-identifier offense. Differential against stock, lint mode.
RSpec.describe "Naming/MethodName edge cases" do
  include EdgeCaseParity

  # `Config#to_h` is the default configuration's internal hash: dup it
  # before reassigning the cop's entry.
  def config_with(cop_options)
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h.dup
    hash["Naming/MethodName"] = hash["Naming/MethodName"].merge(cop_options)
    RuboCop::Config.new(hash, default.loaded_path)
  end

  let(:source) do
    <<~RUBY
      class Foo
        attr_reader :__send__

        def __id__
          1
        end

        def fine; end

        def badName; end

        alias __object_id__ object_id
      end
    RUBY
  end

  it "reports the forbidden identifiers next to the style offenses" do
    config = config_with("ForbiddenIdentifiers" => %w[__id__ __send__ __object_id__])
    offenses = expect_lint_parity(RuboCop::Cop::Naming::MethodName, Shirobai::Cop::Naming::MethodName,
                                  source, config)
    messages = offenses.map { |o| o[2].delete_prefix("Naming/MethodName: ") }
    expect(messages).to contain_exactly("`__send__` is forbidden, use another method name instead.",
                                        "`__id__` is forbidden, use another method name instead.",
                                        "Use snake_case for method names.",
                                        "`__object_id__` is forbidden, use another method name instead.")
  end

  it "still takes the bundle path with only ForbiddenIdentifiers configured" do
    config = config_with("ForbiddenIdentifiers" => %w[__id__])
    cop = Shirobai::Cop::Naming::MethodName.new(config)
    expect(cop.send(:bundle_eligible?)).to be(true)

    config = config_with("ForbiddenPatterns" => ["\\A__"])
    cop = Shirobai::Cop::Naming::MethodName.new(config)
    expect(cop.send(:bundle_eligible?)).to be(false)
  end

  it "matches stock when nothing is forbidden" do
    expect_lint_parity(RuboCop::Cop::Naming::MethodName, Shirobai::Cop::Naming::MethodName,
                       source, config_with({}))
  end
end
