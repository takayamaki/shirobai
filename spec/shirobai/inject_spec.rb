# frozen_string_literal: true

require "spec_helper"

# RuboCop 1.89 registers stock cops lazily: the registry maps each badge to a
# constant-name String, and resolving the constant (autoload) loads the file,
# whose class definition auto-enlists the stock class into the enrollment
# queue. The queue flush is last-write-wins, so a stock counterpart loaded
# AFTER a wrapper claimed the badge would steal the badge back on the next
# flush. `Inject.claim_badges!` prevents this by consuming every replaced
# cop's autoload first and re-enlisting the wrappers after.
RSpec.describe Shirobai::Inject do
  let(:registry) { RuboCop::Cop::Registry.global }

  def wrappers
    described_class.wrapper_cops.select { |w| described_class.stock_counterpart(w) }
  end

  describe "badge ownership" do
    it "resolves every implemented cop name to its shirobai wrapper" do
      wrappers.each do |wrapper|
        owner = registry.find_by_cop_name(wrapper.cop_name)
        expect(owner).to eq(wrapper),
                         "#{wrapper.cop_name}: expected #{wrapper}, registry has #{owner}"
      end
    end

    it "keeps wrapper ownership after the whole registry is materialized" do
      registry.cops # load_all_lazy_cops + queue flush
      wrappers.each do |wrapper|
        owner = registry.find_by_cop_name(wrapper.cop_name)
        expect(owner).to eq(wrapper),
                         "#{wrapper.cop_name}: expected #{wrapper}, registry has #{owner}"
      end
    end
  end

  describe "stock counterpart inoculation" do
    it "has consumed every replaced cop's autoload at require time" do
      wrappers.each do |wrapper|
        dept, name = wrapper.cop_name.split("/")
        mod = RuboCop::Cop.const_get(dept, false)
        expect(mod.autoload?(name.to_sym)).to be_nil,
                                              "#{wrapper.cop_name}: stock autoload still pending"
      end
    end
  end

  describe "lazy loading preservation" do
    it "does not load unreplaced stock cop files at require time" do
      # Separate process: $LOADED_FEATURES in this suite is polluted by every
      # spec that references a stock class. The sentinel is an unreplaced cop
      # that nothing loads unless something forces the whole registry.
      lib = File.expand_path("../../lib", __dir__)
      script = <<~RUBY
        require "rubocop"
        require "shirobai"
        stock = $LOADED_FEATURES.grep(%r{/rubocop/cop/})
        exit 1 if stock.grep(%r{/style/yoda_condition\\.rb$}).any?
        exit 2 if stock.grep(%r{/style/mutable_constant\\.rb$}).empty?
        exit 0
      RUBY
      out = IO.popen([RbConfig.ruby, "-I", lib, "-e", script], err: %i[child out], &:read)
      status = $CHILD_STATUS || $?
      expect(status.exitstatus).to eq(0),
                                   "exit #{status.exitstatus} (1=sentinel loaded, 2=counterpart not loaded)\n#{out}"
    end
  end
end
