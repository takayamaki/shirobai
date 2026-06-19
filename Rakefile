# frozen_string_literal: true

require "rbconfig"

DLEXT = RbConfig::CONFIG["DLEXT"]

task :compile do
  sh "cargo build --release -p shirobai-ext"
  mkdir_p "lib/shirobai"
  cp "target/release/libshirobai.#{DLEXT}", "lib/shirobai/shirobai.#{DLEXT}"
end

task default: :compile

desc "Build and push gem to RubyGems.org (used by release-gem action)"
task :release do
  gemspec = Gem::Specification.load("shirobai.gemspec")
  gem_file = Gem::Package.build(gemspec)
  sh "gem", "push", gem_file
end
