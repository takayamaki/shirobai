# frozen_string_literal: true

# Per-cop isolated benchmark.
#
# Measures the wall time spent running a SINGLE cop over every Mastodon source
# file, comparing the stock RuboCop implementation against the shirobai (Rust)
# drop-in. Sources are parsed once up front and shared; only the
# `Commissioner#investigate` loop is timed. The Commissioner is built once and
# reused across files (rebuilding it per file would add `build_callbacks`
# overhead, per the project notes).
#
# Usage:
#   ruby benches/incremental_bench.rb <stock|shirobai> "Lint/Debugger"
#
# Run each mode in a SEPARATE process (see run_bench.sh) to avoid JIT warming
# bias from running stock and shirobai back to back in one process.

require "benchmark"
require "rubocop"

$LOAD_PATH.unshift File.join(__dir__, "..", "lib")
require "shirobai"

mode = ARGV[0] or abort "usage: incremental_bench.rb <stock|shirobai> <Cop/Name>"
cop_name = ARGV[1] or abort "usage: incremental_bench.rb <stock|shirobai> <Cop/Name>"

config = RuboCop::ConfigLoader.default_configuration
ruby_version = RuboCop::TargetRuby::DEFAULT_VERSION

cop_class =
  case mode
  when "stock"
    RuboCop::Cop::Registry.global.find { |c| c.cop_name == cop_name } or
      abort "unknown stock cop: #{cop_name}"
  when "shirobai"
    Shirobai::Cop.constants(false).filter_map do |dept|
      mod = Shirobai::Cop.const_get(dept)
      next unless mod.is_a?(Module)

      mod.constants(false).map { |name| mod.const_get(name) }
         .find { |klass| klass.respond_to?(:cop_name) && klass.cop_name == cop_name }
    end.compact.first or abort "unknown shirobai cop: #{cop_name}"
  else
    abort "mode must be stock or shirobai"
  end

files = Dir.glob(File.join(__dir__, "..", ".tmp", "mastodon", "**", "*.rb"))
files = files.first(Integer(ENV["LIMIT"])) if ENV["LIMIT"]

# Parse once, share across modes (parse cost is identical for both).
sources = files.filter_map do |path|
  src = File.read(path)
  RuboCop::ProcessedSource.new(src, ruby_version, path)
rescue StandardError
  nil
end

cop = cop_class.new(config)
commissioner = RuboCop::Cop::Commissioner.new([cop])

# Warm up: run a handful of files so the first-call setup is not timed.
sources.first(50).each { |ps| commissioner.investigate(ps) }

offenses = 0
elapsed = Benchmark.realtime do
  sources.each do |ps|
    report = commissioner.investigate(ps)
    offenses += report.offenses.size
  end
end

printf("%-9s %-28s files=%-5d offenses=%-6d %.3fs\n",
       mode, cop_name, sources.size, offenses, elapsed)
