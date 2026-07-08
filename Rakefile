# frozen_string_literal: true

require "rbconfig"
require "rubygems/package"

DLEXT = RbConfig::CONFIG["DLEXT"]

task :compile do
  sh "cargo build --release -p shirobai-ext"
  mkdir_p "lib/shirobai"
  cp "target/release/libshirobai.#{DLEXT}", "lib/shirobai/shirobai.#{DLEXT}"
end

task default: :compile

# Release order: core first, then the plugins. Each plugin gemspec pins
# `shirobai = <same version>`, so core must reach RubyGems.org before the
# plugins are pushed (be conservative about index propagation).
GEMS = [
  { name: "shirobai",             dir: ".",                         gemspec: "shirobai.gemspec" },
  { name: "shirobai-performance", dir: "gems/shirobai-performance", gemspec: "shirobai-performance.gemspec" },
  { name: "shirobai-rspec",       dir: "gems/shirobai-rspec",       gemspec: "shirobai-rspec.gemspec" },
  { name: "shirobai-rails",       dir: "gems/shirobai-rails",       gemspec: "shirobai-rails.gemspec" }
].freeze

# Build one gemspec into the repo-root pkg/. Plugin gemspecs use paths
# relative to their own directory (require_relative + Dir["lib/**/*.rb"]),
# so build from that directory, then move the .gem up to the root pkg/.
def build_gem(dir, gemspec)
  pkg = File.expand_path("pkg", __dir__)
  mkdir_p pkg
  Dir.chdir(dir) do
    spec = Gem::Specification.load(gemspec)
    gem_file = Gem::Package.build(spec)
    dest = File.join(pkg, gem_file)
    mv gem_file, dest
    dest
  end
end

desc "Build all four gems into pkg/ (core first, then the plugins)"
task :build do
  GEMS.each { |g| build_gem(g[:dir], g[:gemspec]) }
end

desc "Build and push all four gems to RubyGems.org (used by release-gem action)"
task :release do
  paths = GEMS.map { |g| build_gem(g[:dir], g[:gemspec]) }
  # Push core, wait for it to be indexed, then push the plugins whose
  # `= version` dependency resolves against it.
  core_path, *plugin_paths = paths
  sh "gem", "push", core_path
  sh "gem", "exec", "rubygems-await", core_path
  plugin_paths.each { |path| sh "gem", "push", path }
end
