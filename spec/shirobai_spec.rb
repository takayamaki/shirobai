# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai do
  it "defines the Shirobai module from native extension" do
    expect(defined?(Shirobai)).to eq("constant")
  end

  it "has a version number" do
    expect(Shirobai::VERSION).to eq("0.1.0")
  end
end
