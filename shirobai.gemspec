# frozen_string_literal: true

require_relative "lib/shirobai/version"

Gem::Specification.new do |spec|
  spec.name = "shirobai"
  spec.version = Shirobai::VERSION
  spec.authors = ["fusagiko / takayamaki"]
  spec.summary = "Drop-in Rust replacement for heavy RuboCop cops"
  spec.homepage = "https://github.com/takayamaki/shirobai"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.1"

  spec.files = Dir["lib/**/*.rb", "ext/**/*.{rs,toml}", "crates/**/*.{rs,toml}", "Cargo.*", "LICENSE.txt"]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/shirobai/Cargo.toml"]

  spec.add_dependency "rubocop", "= 1.88.0"
end
