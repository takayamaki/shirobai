# frozen_string_literal: true

require "spec_helper"

# Regression guard for CRLF sources.
#
# `ProcessedSource#raw_source` keeps every "\r", but parser-gem builds
# `buffer.source` with "\r\n" replaced by "\n", and every stock range (AST,
# tokens, `lines`) indexes that normalized string. If Rust scans `raw_source`,
# each offset after the first line break is ahead by the number of "\r" before
# it, so offenses land on the wrong columns or lines. The LF corpora cannot
# see this, so these examples run the stock cop and the shirobai cop side by
# side over CRLF copies of real files.
RSpec.describe "CRLF source offset parity with stock RuboCop" do
  let(:config) { RuboCop::ConfigLoader.default_configuration }

  def stdlib_source(name)
    File.read(File.join(RbConfig::CONFIG["rubylibdir"], name))
  end

  def to_crlf(source)
    source.gsub(/\r?\n/, "\r\n")
  end

  # Runs `klass` over `source` in autocorrect mode with the vendor-spec
  # iteration semantics (same cop instance across passes, loop until the
  # corrector is empty or a fixpoint). Returns the first-pass offense
  # snapshots and the final source.
  def autocorrect_run(klass, source, config)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first_offenses = nil
    11.times do |iteration|
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
      raise "autocorrect loop did not converge" if iteration == 10

      src = rewritten
    end
    [first_offenses, src]
  end

  def implemented_cops
    Shirobai::Cop.constants(false).sort.flat_map do |department|
      mod = Shirobai::Cop.const_get(department)
      next [] unless mod.is_a?(Module)

      mod.constants(false).sort.filter_map do |name|
        klass = mod.const_get(name)
        klass if klass.is_a?(Class) && klass < RuboCop::Cop::Base
      end
    end
  end

  # Like `autocorrect_run`, but an exception becomes part of the result, so
  # one crashing cop does not hide the rest of the sweep.
  def outcome(klass, source, config)
    autocorrect_run(klass, source, config)
  rescue StandardError => e
    [:raised, e.class]
  end

  # The names of the implemented cops whose first-pass offenses or final
  # autocorrected source differ from stock on `source`.
  def diverging_cops(source, config)
    implemented_cops.filter_map do |klass|
      department, name = klass.cop_name.split("/")
      stock_klass = RuboCop::Cop.const_get(department).const_get(name)
      next if outcome(stock_klass, source, config) == outcome(klass, source, config)

      klass.cop_name
    end
  end

  describe "an ASCII-only CRLF file (Ruby stdlib resolv.rb)" do
    let(:source) { to_crlf(stdlib_source("resolv.rb")) }

    it "is ASCII-only and has CRLF line endings (precondition for the sweep)" do
      expect(source.ascii_only?).to be(true)
      expect(source.count("\r")).to eq(source.count("\n"))
    end

    it "matches stock offenses and autocorrect output for every implemented cop" do
      expect(diverging_cops(source, config)).to eq([])
    end
  end

  describe "a CRLF file with multibyte characters (Ruby stdlib fileutils.rb)" do
    let(:source) { to_crlf(stdlib_source("fileutils.rb")) }

    it "is non-ASCII and has CRLF line endings (precondition for the sweep)" do
      expect(source.ascii_only?).to be(false)
      expect(source.count("\r")).to eq(source.count("\n"))
    end

    it "matches stock offenses and autocorrect output for every implemented cop" do
      expect(diverging_cops(source, config)).to eq([])
    end
  end

  # Every other line ends in CRLF, so the "\r" count before an offset is not
  # a simple function of its line number.
  def to_mixed(source)
    source.lines.each_with_index.map do |line, index|
      index.even? ? line.sub(/\r?\n\z/, "\r\n") : line
    end.join
  end

  describe "a file that mixes CRLF and LF line endings" do
    let(:source) { to_mixed(stdlib_source("fileutils.rb")) }

    it "matches stock offenses and autocorrect output for every implemented cop" do
      expect(source).to include("\r\n").and match(/[^\r]\n/)
      expect(diverging_cops(source, config)).to eq([])
    end
  end

  # Layout/EndOfLine reads `raw_source` on purpose (stock does too): the "\r"
  # it looks for is gone from `buffer.source`.
  describe "cops that read the raw line endings" do
    def end_of_line_config(style)
      RuboCop::ConfigLoader.merge_with_default(
        RuboCop::Config.new({ "Layout/EndOfLine" => { "EnforcedStyle" => style } }, "(test)"),
        "(test)"
      )
    end

    def expect_end_of_line_parity(source, style)
      cfg = end_of_line_config(style)
      stock = outcome(RuboCop::Cop::Layout::EndOfLine, source, cfg)
      expect(outcome(Shirobai::Cop::Layout::EndOfLine, source, cfg)).to eq(stock)
      expect(stock.first).not_to be_empty, "fixture produced no stock offense; fix the source"
    end

    it "matches stock Layout/EndOfLine with EnforcedStyle: crlf on a mixed file" do
      expect_end_of_line_parity(to_mixed(stdlib_source("resolv.rb")), "crlf")
    end

    it "matches stock Layout/EndOfLine with EnforcedStyle: lf on a CRLF file" do
      expect_end_of_line_parity(to_crlf(stdlib_source("resolv.rb")), "lf")
    end
  end
end
