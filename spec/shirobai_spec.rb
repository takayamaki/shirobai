# frozen_string_literal: true

require "spec_helper"

RSpec.describe Shirobai do
  it "defines the Shirobai module from native extension" do
    expect(defined?(Shirobai)).to eq("constant")
  end

  it "has a version number in calendar format" do
    expect(Shirobai::VERSION).to match(/\A\d{4}\.\d{3,4}\.\d{3,4}\z/)
  end
end
