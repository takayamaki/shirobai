# frozen_string_literal: true

require "spec_helper"

# rubocop-rspec half of the core suite's spec/shirobai/crlf_path_parity_spec.rb:
# a CRLF spec file must take the same path as its LF original.
RSpec.describe "CRLF source path parity (RSpec cops)" do
  include StandalonePathParity

  let(:cops) { wrapper_cops(%i[RSpec]) }
  let(:config) { config_enabling(cops) }

  # One spec file that reaches every wrapper's Rust call.
  let(:source) do
    <<~RUBY
      # frozen_string_literal: true

      RSpec.describe Foo, :b, :a, type: :model do
        let(:user_1) { Foo.new }
        let('name') { "x" }
        subject(:foo) { Foo.new }
        let(:unused) { create(:thing) }
        let!(:setup) { create(:other) }
        before { prepare }
        it { expect(foo).to be_valid }
        it { expect(foo).to be_valid }
        it "does a thing" do
          expect(subject).to eq(Foo.new)
        end
        it "does a thing" do
          expect(user_1).to be
        end
        before { more }
        context "empty" do
        end
        fit "is focused", focus: true, skip: true do
          pending
        end
        describe "metadata", :b, :a, :a, {} do
          it "runs" do
            Foo.call
          end
        end
        after { cleanup }
        it "last" do
          expect(1).to eq(1)
        end
      end
    RUBY
  end

  it "calls the same standalone entry points on a CRLF copy of a spec file" do
    expect(path_diverging_cops(cops, source, config)).to eq({})
  end
end
