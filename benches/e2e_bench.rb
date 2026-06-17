# frozen_string_literal: true

# End-to-end benchmark: run cops over every .rb file in a corpus,
# using the corpus's own .rubocop.yml config.
#
# Cop instances and the Commissioner are built FRESH PER FILE,
# matching RuboCop's Runner#inspect_file -> mobilize_team behavior.
# Do not reuse instances: stock cops leak state across files.
#
# Usage:
#   ruby benches/e2e_bench.rb <stock|shirobai|removed> [corpus-path]
#
# corpus-path defaults to .tmp/mastodon.
# Run each mode in a separate process (see run_e2e.sh).

require "rubocop"

mode = ARGV[0] or abort "usage: e2e_bench.rb <stock|shirobai|removed> [corpus-path]"
corpus = ARGV[1] || File.join(__dir__, "..", ".tmp", "mastodon")
corpus = File.expand_path(corpus)

abort "error: corpus not found: #{corpus}" unless Dir.exist?(corpus)

shirobai_cops = {}
if mode != "stock"
  $LOAD_PATH.unshift File.join(__dir__, "..", "lib")
  require "shirobai"

  Shirobai::Cop.constants(false).each do |dept|
    mod = Shirobai::Cop.const_get(dept)
    next unless mod.is_a?(Module)

    mod.constants(false).each do |name|
      klass = mod.const_get(name)
      next unless klass.is_a?(Class) && klass < RuboCop::Cop::Base

      shirobai_cops[klass.cop_name] = klass
    end
  end
end

# Load the corpus's .rubocop.yml but skip require/inherit_gem/plugins
# so we don't need to install every plugin gem the corpus uses.
# Only default cops (shirobai's replacement targets) matter for this bench.
config_file = RuboCop::ConfigLoader.configuration_file_for(corpus)
config = if File.exist?(config_file)
           hash = RuboCop::ConfigLoader.load_yaml_configuration(config_file)
           %w[require inherit_gem inherit_from plugins].each { |k| hash.delete(k) }
           RuboCop::ConfigLoader.merge_with_default(
             RuboCop::Config.create(hash, config_file, check: false),
             config_file
           )
         else
           RuboCop::ConfigLoader.default_configuration
         end
ruby_version = config.target_ruby_version || RuboCop::TargetRuby::DEFAULT_VERSION
registry = RuboCop::Cop::Registry.global

cop_classes = registry.enabled(config).filter_map do |klass|
  implemented = shirobai_cops.key?(klass.cop_name)
  case mode
  when "shirobai" then implemented ? shirobai_cops[klass.cop_name] : klass
  when "removed" then implemented ? nil : klass
  else klass
  end
end

replaced = mode == "shirobai" ? shirobai_cops.keys.size : 0

files = Dir.glob(File.join(corpus, "**", "*.rb"))
files = files.first(Integer(ENV["LIMIT"])) if ENV["LIMIT"]

sources = files.filter_map do |path|
  ps = RuboCop::ProcessedSource.new(File.read(path), ruby_version, path)
  ps.config = config
  ps.registry = registry
  ps
rescue StandardError
  nil
end

investigate = lambda do |ps|
  cops = cop_classes.map { |klass| klass.new(config) }
  RuboCop::Cop::Commissioner.new(cops).investigate(ps)
end

sources.first(50).each { |ps| investigate.call(ps) }
GC.start

offenses = 0
gc0 = GC.stat(:time)
c0 = Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID)
w0 = Process.clock_gettime(Process::CLOCK_MONOTONIC)
sources.each do |ps|
  offenses += investigate.call(ps).offenses.size
end
cpu = Process.clock_gettime(Process::CLOCK_PROCESS_CPUTIME_ID) - c0
wall = Process.clock_gettime(Process::CLOCK_MONOTONIC) - w0
gc = (GC.stat(:time) - gc0) / 1000.0
compute = cpu - gc

printf("%-9s cops=%-4d replaced=%-2d files=%-5d offenses=%-7d " \
       "cpu=%.2fs compute=%.2fs gc=%.2fs wall=%.2fs\n",
       mode, cop_classes.size, replaced, sources.size, offenses, cpu, compute, gc, wall)
