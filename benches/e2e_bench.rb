# frozen_string_literal: true

# End-to-end benchmark: run the FULL default cop set over every Mastodon file,
# comparing stock RuboCop against shirobai (with the implemented cops swapped
# in). This is the metric that matters — in a real run the AST walk is shared
# across all cops, so the per-cop isolated benchmark cannot show shirobai's
# benefit.
#
# Sources are parsed once up front and shared; only the investigate loop is
# timed. Cop instances and the Commissioner are built FRESH PER FILE, exactly
# like RuboCop's Runner#inspect_file -> mobilize_team does in a real run. Do
# not "optimize" this by reusing instances: stock cops are stateful across
# investigations (e.g. Layout/LineLength leaks @heredocs line ranges between
# files, silently suppressing offenses), so a reused-instance run measures a
# corrupted workload that no real rubocop invocation ever executes.
#
# Usage:
#   ruby benches/e2e_bench.rb <stock|shirobai|removed>
#
# Run each mode in a separate process (see run_e2e.sh).

require "rubocop"

mode = ARGV[0] or abort "usage: e2e_bench.rb <stock|shirobai|removed>"

# In stock mode shirobai must NOT be loaded at all: requiring it enlists the
# wrapper classes into the global registry under the same badges, replacing the
# stock cops — exactly the drop-in behavior the gem ships, but fatal for a
# baseline (stock would silently run the wrappers and stock-vs-shirobai becomes
# a self-comparison). removed/shirobai modes still need the require to know the
# implemented cop set.
shirobai_cops = {}
if mode != "stock"
  $LOAD_PATH.unshift File.join(__dir__, "..", "lib")
  require "shirobai"

  # Map of cop_name => shirobai class, for the cops implemented so far.
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

config = RuboCop::ConfigLoader.default_configuration
ruby_version = RuboCop::TargetRuby::DEFAULT_VERSION
registry = RuboCop::Cop::Registry.global

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

replaced = mode == "shirobai" ? shirobai_cops.keys.size : 0

files = Dir.glob(File.join(__dir__, "..", ".tmp", "mastodon", "**", "*.rb"))
files = files.first(Integer(ENV["LIMIT"])) if ENV["LIMIT"]

sources = files.filter_map do |path|
  RuboCop::ProcessedSource.new(File.read(path), ruby_version, path)
rescue StandardError
  nil
end

# Mirror Runner#inspect_file: fresh cop instances + fresh Commissioner per
# file (see header comment for why reuse corrupts stock behavior).
investigate = lambda do |ps|
  cops = cop_classes.map { |klass| klass.new(config) }
  RuboCop::Cop::Commissioner.new(cops).investigate(ps)
end

sources.first(50).each { |ps| investigate.call(ps) }

# Sweep warmup-era garbage before starting the clock so it is not collected
# (and charged) inside the timed region. Do NOT GC.disable: shirobai's value is
# reduced Ruby allocation/eval, and that shows up as reduced GC cost, so GC must
# be inside the measured region.
GC.start

offenses = 0
# Measure process CPU time (utime+stime), not wall time. The investigate loop is
# effectively single-threaded (manual Commissioner loop + synchronous magnus
# calls), so process CPU time ~= the pure computational cost and drops the
# scheduling/contention jitter that inflates wall time. Ruby eval, Rust walk and
# reparse are all on-CPU user time in the same process, so stock-vs-shirobai
# stays a fair compute comparison. Wall is kept as a sanity check: cpu << wall
# means external load contaminated the run.
#
# We also split CPU into GC vs compute. GC time (GC.stat(:time), cumulative ms)
# is the dominant run-to-run noise source: stock allocates far more Ruby objects
# than shirobai (whose cops do the work in Rust and return compact tuples), so
# stock's GC time both is larger and swings more. `compute = cpu - gc` is the
# low-variance signal for "is the actual analysis faster"; `gc` is reported on
# its own because shirobai's reduced allocation is a real (if noisy) part of its
# win, so we must not hide it (no GC.disable).
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
