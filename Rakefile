# frozen_string_literal: true

require "rubygems/package"
require "rb_sys/extensiontask"

# Drives the native build through rb_sys/rake-compiler so the same
# `create_rust_makefile` path used by `gem install` (extconf.rb) also runs
# under `rake compile`. The compiled cdylib lands at lib/shirobai/shirobai.so,
# which CI's build job uploads and the loader in lib/shirobai.rb requires.
GEMSPEC = Gem::Specification.load("shirobai.gemspec")

RbSys::ExtensionTask.new("shirobai", GEMSPEC) do |ext|
  ext.lib_dir = "lib/shirobai"
  ext.cross_compile = true
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

# True when this exact version of the gem is already on RubyGems.org.
# Lets `rake release` re-run after a partial failure: pushes that already
# went through are skipped instead of failing the whole task.
def published?(name, version)
  require "json"
  require "net/http"
  res = Net::HTTP.get_response(URI("https://rubygems.org/api/v1/versions/#{name}.json"))
  return false unless res.is_a?(Net::HTTPSuccess)

  JSON.parse(res.body).any? { |v| v["number"] == version }
end

desc "Build and push all four gems to RubyGems.org (used by release-gem action)"
task :release do
  require_relative "lib/shirobai/version"
  version = Shirobai::VERSION
  pairs = GEMS.map { |g| [g[:name], build_gem(g[:dir], g[:gemspec])] }
  # Push core, wait for it to be indexed, then push the plugins whose
  # `= version` dependency resolves against it.
  (core_name, core_path), *plugins = pairs
  sh "gem", "push", core_path unless published?(core_name, version)
  # `gem exec` must run outside the bundle: under `bundle exec rake`,
  # Bundler blocks any gem that is not in the Gemfile.
  Bundler.with_unbundled_env do
    sh "gem", "exec", "rubygems-await", core_path
  end
  plugins.each { |name, path| sh("gem", "push", path) unless published?(name, version) }
end
