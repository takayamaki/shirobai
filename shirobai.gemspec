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

  # `.rb` under ext/ must be shipped too: extconf.rb is the build entry point,
  # so omitting it makes `gem install` fail with no extension to compile.
  spec.files = Dir["lib/**/*.rb", "ext/**/*.{rs,toml,rb}", "crates/**/*.{rs,toml}", "Cargo.*", "LICENSE.txt"]
  spec.require_paths = ["lib"]
  spec.extensions = ["ext/shirobai/extconf.rb"]

  spec.add_dependency "rb_sys", "~> 0.9"
  spec.add_dependency "rubocop", "= 1.88.2"
end
