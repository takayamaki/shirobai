# frozen_string_literal: true

# Helpers that pin WHICH path a wrapper cop takes, not just its output.
#
# Every wrapper reads its offenses from the shared walk (`Shirobai.check_all`
# through `Shirobai::Dispatch`). Some wrappers also call a standalone
# `Shirobai.check_*` entry point on purpose on every file. A CRLF file must
# take the same path as its LF original: the line endings are not a reason
# to leave the shared walk, since both paths scan `buffer.source`.
#
# The output is the same on both paths, so an output test cannot catch a cop
# that quietly falls back to its standalone entry point on CRLF files (it is
# only slower). These helpers count the standalone calls instead.
module StandalonePathParity
  # Ext methods that are not standalone cop entry points.
  NON_ENTRY_METHODS = %i[check_all register_bundle_config].freeze

  def self.entry_methods
    Shirobai.singleton_methods.sort - NON_ENTRY_METHODS
  end

  def to_crlf(source)
    source.gsub(/\r?\n/, "\r\n")
  end

  # Every other line ends in CRLF. Some cops (Layout/LineContinuationSpacing)
  # look for a backslash right before "\n", so on an all-CRLF file they stop
  # early and never reach the branch this guards.
  def to_mixed(source)
    source.lines.each_with_index.map do |line, index|
      index.even? ? line.sub(/\r?\n\z/, "\r\n") : line
    end.join
  end

  # Every wrapper cop class under `Shirobai::Cop::<department>`, optionally
  # limited to the given department names.
  def wrapper_cops(departments = nil)
    Shirobai::Cop.constants(false).sort.flat_map do |department|
      next [] if departments && !departments.include?(department)

      mod = Shirobai::Cop.const_get(department)
      next [] unless mod.is_a?(Module)

      mod.constants(false).sort.filter_map do |name|
        klass = mod.const_get(name)
        klass if klass.is_a?(Class) && klass < RuboCop::Cop::Base
      end
    end
  end

  # `base` with every cop in `cops` enabled, so `pending` and disabled cops
  # run too (Team skips a cop its config does not enable).
  def config_enabling(cops, base = RuboCop::ConfigLoader.default_configuration)
    hash = base.to_h.dup
    cops.each do |klass|
      hash[klass.cop_name] = (hash[klass.cop_name] || {}).merge("Enabled" => true)
    end
    RuboCop::Config.new(hash, base.loaded_path)
  end

  # Spies on every standalone entry point and returns the counts Hash.
  # The spies call the real method, so the cops still get real results.
  def spy_on_entry_points
    counts = Hash.new(0)
    StandalonePathParity.entry_methods.each do |name|
      allow(Shirobai).to receive(name).and_wrap_original do |original, *args|
        counts[name] += 1
        original.call(*args)
      end
    end
    counts
  end

  # Runs `klass` once over `source` (autocorrect on, so autocorrect-only
  # paths run too) and returns the standalone calls it made, by method.
  def standalone_calls(klass, source, config, counts)
    counts.clear
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    processed = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    RuboCop::Cop::Team.new([cop], config, raise_error: true).investigate(processed)
    counts.reject { |_, n| n.zero? }
  end

  # The names of the cops whose standalone calls on `crlf` (by default the
  # CRLF copy of `source`) differ from those on the LF `source` itself, with
  # both call counts.
  def path_diverging_cops(cops, source, config, crlf: to_crlf(source))
    counts = spy_on_entry_points
    cops.each_with_object({}) do |klass, diverging|
      lf_calls = standalone_calls(klass, source, config, counts)
      crlf_calls = standalone_calls(klass, crlf, config, counts)
      diverging[klass.cop_name] = { lf: lf_calls, crlf: crlf_calls } unless lf_calls == crlf_calls
    end
  end
end
