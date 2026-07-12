# frozen_string_literal: true

# Shared differential helpers for the per-cop edge-case regression specs
# (`spec/shirobai/cop/<dept>/<name>_edge_cases_spec.rb`).
#
# These specs pin "edge-case behaviours the VENDOR specs do not exercise" that
# were discovered the hard way by stock real-machine probing / corpus
# divergence (prism write-target bypass, lambda-as-block-node, elsif shared
# `end`, hash clobber confinement, multi-pass ignored-range accumulation,
# branch-hook non-firing keyword completion). The vendor specs do not guard
# them and the corpus parity is disposable, so a refactor could silently
# regress them.
#
# Every helper runs the STOCK cop and the SHIROBAI cop side by side over the
# same source, with stock generated FRESH per file/iteration (no instance
# reuse, matching the real CLI's `Runner#inspect_file` -> `mobilize_team`), and
# asserts identical results. "Both produce no offense" is a valid assertion
# (e.g. the write-target bypass: stock and shirobai must both stay at zero so
# shirobai does not false-positive). Cases that DO expect offenses also assert
# stock produced at least one, so a mistyped fixture cannot pass vacuously.
module EdgeCaseParity
  # Lint mode (a bare Commissioner, no autocorrect option). Returns offense
  # snapshots down to begin/end position, message, status, correctable?.
  def lint_offenses(klass, source, config)
    cop = klass.new(config)
    processed = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION)
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    report = RuboCop::Cop::Commissioner.new([cop]).investigate(processed)
    expect(report.errors).to be_empty
    report.offenses.map do |o|
      [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
    end.sort
  end

  # Lint-mode differential: stock and shirobai must agree exactly. When
  # `expect_offenses: true` (the default), also assert stock produced at least
  # one offense so the comparison is not vacuous. Returns the stock snapshot.
  def expect_lint_parity(stock_klass, shirobai_klass, source, config, expect_offenses: true)
    stock = lint_offenses(stock_klass, source, config)
    if expect_offenses
      expect(stock).not_to be_empty, "fixture produced no stock offense; fix the source"
    end
    expect(lint_offenses(shirobai_klass, source, config)).to eq(stock)
    stock
  end

  # Autocorrect to convergence with the vendor-spec iteration semantics (one cop
  # instance across passes, loop until the corrector is empty or a fixpoint).
  # Returns [first_pass_offenses, final_source]. `max` iterations guards against
  # an oscillating (non-converging) autocorrect.
  #
  # `fresh_cop_per_pass: true` builds a new cop instance for every pass, which
  # is what the real CLI does (the Runner mobilizes a fresh team per correction
  # round). Needed for cops that leak state across investigations — stock
  # Layout/LineLength never resets `@breakable_range_by_line_index`, so a
  # reused instance crashes ("Correction target buffer ... is not current")
  # whenever a later pass registers an offense on a line whose claimed range
  # came from the previous pass's buffer.
  def autocorrect_run(klass, source, config, max: 11, fresh_cop_per_pass: false)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first_offenses = nil
    max.times do |iteration|
      if fresh_cop_per_pass && iteration.positive?
        cop = klass.new(config)
        cop.instance_variable_get(:@options)[:autocorrect] = true
      end
      processed = RuboCop::ProcessedSource.new(src, RuboCop::TargetRuby::DEFAULT_VERSION)
      processed.config = config
      processed.registry = RuboCop::Cop::Registry.global
      team = RuboCop::Cop::Team.new([cop], config, raise_error: true)
      report = team.investigate(processed)
      offenses = report.offenses.map do |o|
        [o.location.begin_pos, o.location.end_pos, o.message, o.status, o.correctable?]
      end.sort
      first_offenses ||= offenses
      corrector = report.correctors.first
      break if corrector.nil? || corrector.empty?

      rewritten = corrector.rewrite
      break if rewritten == src
      raise "autocorrect loop did not converge" if iteration == max - 1

      src = rewritten
    end
    [first_offenses, src]
  end

  # One CLI-like correction round: a fresh team of `cop_classes` (in the
  # given order), autocorrect on, single investigate. Returns the rewritten
  # source (via the stdin path). Unlike the single-cop helpers, this
  # exercises `Team#autocorrect`'s corrector merge — clobber drops and
  # `autocorrect_incompatible_with` skips — across several cops.
  def one_team_round(cop_classes, source, config, path: nil)
    options = { autocorrect: true, stdin: source.dup, raise_error: true }
    cops = cop_classes.map { |klass| klass.new(config, options) }
    team = RuboCop::Cop::Team.new(cops, config, options)
    # `path` matters for cops with a department `Include` (RSpec cops only
    # run on `**/*_spec.rb`); pass an absolute spec-like path for those.
    processed = RuboCop::ProcessedSource.new(source, RuboCop::TargetRuby::DEFAULT_VERSION, path)
    processed.config = config
    processed.registry = RuboCop::Cop::Registry.global
    team.investigate(processed)
    options[:stdin]
  end

  # Autocorrect differential: stock and shirobai must agree on both the
  # first-pass offenses and the fully corrected source. Returns the stock
  # corrected source.
  def expect_autocorrect_parity(stock_klass, shirobai_klass, source, config,
                                fresh_cop_per_pass: false)
    stock_offenses, stock_corrected =
      autocorrect_run(stock_klass, source, config, fresh_cop_per_pass: fresh_cop_per_pass)
    shirobai_offenses, shirobai_corrected =
      autocorrect_run(shirobai_klass, source, config, fresh_cop_per_pass: fresh_cop_per_pass)
    expect(shirobai_offenses).to eq(stock_offenses)
    expect(shirobai_corrected).to eq(stock_corrected)
    stock_corrected
  end
end
