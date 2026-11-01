# frozen_string_literal: true

require "spec_helper"

# rubocop-performance half of the core suite's
# spec/shirobai/crlf_path_parity_spec.rb: a CRLF file must take the same path
# as its LF original.
RSpec.describe "CRLF source path parity (Performance cops)" do
  include StandalonePathParity

  let(:cops) { wrapper_cops(%i[Performance]) }
  let(:config) { config_enabling(cops) }

  let(:source) do
    <<~RUBY
      # frozen_string_literal: true

      items.select { |x| x > 1 }.first
      items.find_all(&:ok?).last
      name.match?(/\\Afoo/)
      name =~ /bar\\z/
      name.match?(/baz/)
      3.times.map { |i| i * 2 }
    RUBY
  end

  it "calls the same standalone entry points on a CRLF copy" do
    expect(path_diverging_cops(cops, source, config)).to eq({})
  end
end
