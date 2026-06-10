# frozen_string_literal: true

# End-to-end benchmark: run the FULL default cop set over every Mastodon file,
# comparing stock RuboCop against shirobai (with the implemented cops swapped
# in). This is the metric that matters — in a real run the AST walk is shared
# across all cops, so the per-cop isolated benchmark cannot show shirobai's
# benefit.
#
# Sources are parsed once up front and shared; only the investigate loop is
# timed. The Commissioner is built once and reused.
#
# Usage:
#   ruby benches/e2e_bench.rb <stock|shirobai>
#
# Run each mode in a separate process (see run_e2e.sh).

require "benchmark"
require "rubocop"

$LOAD_PATH.unshift File.join(__dir__, "..", "lib")
require "shirobai"

mode = ARGV[0] or abort "usage: e2e_bench.rb <stock|shirobai>"

config = RuboCop::ConfigLoader.default_configuration
ruby_version = RuboCop::TargetRuby::DEFAULT_VERSION
registry = RuboCop::Cop::Registry.global

# Map of cop_name => shirobai class, for the cops implemented so far.
shirobai_cops = {}
Shirobai::Cop.constants(false).each do |dept|
  mod = Shirobai::Cop.const_get(dept)
  next unless mod.is_a?(Module)

  mod.constants(false).each do |name|
    klass = mod.const_get(name)
    next unless klass.is_a?(Class) && klass < RuboCop::Cop::Base

    shirobai_cops[klass.cop_name] = klass
  end
end

# Modes:
#   stock    - all default cops, unchanged
#   shirobai - implemented cops swapped for the Rust drop-ins
#   removed  - implemented cops dropped entirely (measures their stock eval cost)
cop_classes = registry.enabled(config).filter_map do |klass|
  implemented = shirobai_cops.key?(klass.cop_name)
  case mode
  when "shirobai" then implemented ? shirobai_cops[klass.cop_name] : klass
  when "removed" then implemented ? nil : klass
  else klass
  end
end

cops = cop_classes.map { |klass| klass.new(config) }
replaced = mode == "shirobai" ? shirobai_cops.keys.size : 0

files = Dir.glob(File.join(__dir__, "..", ".tmp", "mastodon", "**", "*.rb"))
files = files.first(Integer(ENV["LIMIT"])) if ENV["LIMIT"]

sources = files.filter_map do |path|
  RuboCop::ProcessedSource.new(File.read(path), ruby_version, path)
rescue StandardError
  nil
end

commissioner = RuboCop::Cop::Commissioner.new(cops)
sources.first(50).each { |ps| commissioner.investigate(ps) }

offenses = 0
elapsed = Benchmark.realtime do
  sources.each do |ps|
    offenses += commissioner.investigate(ps).offenses.size
  end
end

printf("%-9s cops=%-4d replaced=%-2d files=%-5d offenses=%-7d %.2fs\n",
       mode, cops.size, replaced, sources.size, offenses, elapsed)
