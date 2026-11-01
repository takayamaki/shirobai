# frozen_string_literal: true

require "spec_helper"

# A CRLF file must take the same path as its LF original: the shared walk,
# plus only the standalone entry points the cop calls on every file anyway.
# See `support/standalone_path_parity.rb` for why this counts calls instead
# of comparing output.
RSpec.describe "CRLF source path parity (core cops)" do
  include StandalonePathParity

  let(:config) { config_enabling(wrapper_cops) }

  def stdlib_source(name)
    File.read(File.join(RbConfig::CONFIG["rubylibdir"], name))
  end

  it "calls the same standalone entry points on a CRLF copy of an ASCII-only file (resolv.rb)" do
    expect(path_diverging_cops(wrapper_cops, stdlib_source("resolv.rb"), config)).to eq({})
  end

  it "calls the same standalone entry points on a CRLF copy of a non-ASCII file (fileutils.rb)" do
    expect(path_diverging_cops(wrapper_cops, stdlib_source("fileutils.rb"), config)).to eq({})
  end

  it "calls the same standalone entry points on a copy that mixes CRLF and LF line endings" do
    source = <<~RUBY
      # frozen_string_literal: true

      total = 1 +\\
        2 + \\
        3 +\\
        4
      puts(total)
    RUBY
    expect(path_diverging_cops(wrapper_cops, source, config, crlf: to_mixed(source))).to eq({})
  end
end
