# frozen_string_literal: true

require_relative "lib/shirobai/rails/version"

Gem::Specification.new do |spec|
  spec.name = "shirobai-rails"
  spec.version = Shirobai::Rails::VERSION
  spec.authors = ["fusagiko / takayamaki"]
  spec.summary = "Drop-in Rust replacement for rubocop-rails cops"
  spec.homepage = "https://github.com/takayamaki/shirobai"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.1"

  # Pure-Ruby thin shell: wrappers + load-order glue only. The Rust rules
  # live in the shirobai core gem's native extension (one shared cdylib).
  spec.files = Dir["lib/**/*.rb", "LICENSE.txt"]
  spec.require_paths = ["lib"]

  # Version lockstep with core: the wire format (bundle slots / packed
  # config segment) is an internal contract between the two gems, so only
  # the exact same version is compatible.
  spec.add_dependency "shirobai", "= #{Shirobai::Rails::VERSION}"

  # Hard pin, same policy as core's rubocop pin: shirobai copies cop
  # behavior at the byte level, so even a minor rubocop-rails update
  # can break compatibility. A failed install beats a silent difference.
  spec.add_dependency "rubocop-rails", "= 2.35.5"
end
