# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/HashSyntax` (rubocop 1.92 behavior the
# vendor spec does not pin down):
#
# - A rocket expansion of an omitted value removes the blanks after the colon
#   but keeps the line break, also when spaces or a tab sit before it.
# - An omitted value named after a keyword (`def m(if:)`) has no explicit form,
#   so no style or shorthand check touches it, across every EnforcedStyle.
# - A no-mixed-keys style that converts the colon pairs to rockets keeps the
#   shorthand checks away from the hash, also under `consistent`.
#
# Differential: stock and shirobai run side by side at Ruby 3.3 and must agree
# on the first-pass offenses and the corrected source.
RSpec.describe "Style/HashSyntax edge cases" do
  let(:stock) { RuboCop::Cop::Style::HashSyntax }
  let(:shirobai) { Shirobai::Cop::Style::HashSyntax }

  def hs_config(style, shorthand)
    base = RuboCop::ConfigLoader.default_configuration
    cop_cfg = base["Style/HashSyntax"].merge(
      "Enabled" => true, "EnforcedStyle" => style, "EnforcedShorthandSyntax" => shorthand
    )
    RuboCop::Config.new(
      {
        "AllCops" => base["AllCops"].merge("TargetRubyVersion" => 3.3),
        "Style/HashSyntax" => cop_cfg
      },
      "#{Dir.pwd}/.rubocop.yml"
    )
  end

  def hs_run(klass, source, config)
    cop = klass.new(config)
    cop.instance_variable_get(:@options)[:autocorrect] = true
    src = source
    first = nil
    11.times do |i|
      ps = RuboCop::ProcessedSource.new(src, 3.3)
      ps.config = config
      ps.registry = RuboCop::Cop::Registry.global
      report = RuboCop::Cop::Team.new([cop], config, raise_error: true).investigate(ps)
      offs = report.offenses.map { |o| [o.location.begin_pos, o.location.end_pos, o.message] }.sort
      first ||= offs
      corr = report.correctors.first
      break if corr.nil? || corr.empty?

      rewritten = corr.rewrite
      break if rewritten == src
      raise "did not converge" if i == 10

      src = rewritten
    end
    [first, src]
  end

  def expect_parity(source, style, shorthand)
    config = hs_config(style, shorthand)
    stock_offs, stock_src = hs_run(stock, source, config)
    expect(stock_offs).not_to be_empty, "fixture produced no stock offense; fix the source"
    sb_offs, sb_src = hs_run(shirobai, source, config)
    expect(sb_offs).to eq(stock_offs)
    expect(sb_src).to eq(stock_src)
    stock_src
  end

  it "keeps the line break after spaces that follow an omitted value" do
    out = expect_parity("cc(\n  bb:   \n)\ndd(aa: 1, bb:\t\n  )\n", "hash_rockets", "always")
    expect(out).to eq("cc(\n  :bb => bb\n)\ndd(:aa => 1, :bb => bb\n  )\n")
  end

  keyword_src = <<~RUBY
    def m(if:, bar:, a:)
      f("s" => 1, bar: bar, a:)
      g(:x => 1, a: a, if:)
      h(a: a, "k" => 2, if:)
      i(if:,
        a:
      )
    end
  RUBY

  %w[ruby19 hash_rockets no_mixed_keys ruby19_no_mixed_keys].each do |style|
    %w[always never consistent].each do |shorthand|
      it "leaves keyword-named omitted values alone (#{style} / #{shorthand})" do
        expect_parity(keyword_src, style, shorthand)
      end
    end
  end

  it "converts to rockets without a shorthand offense under consistent" do
    src = "def m(if:, bar:, a:)\n  f(\"s\" => 1, bar:, a: a, if:)\n  g(bar:, a: 1, if:)\nend\n"
    out = expect_parity(src, "ruby19_no_mixed_keys", "consistent")
    expect(out).to eq(
      "def m(if:, bar:, a:)\n  f(\"s\" => 1, :bar => bar, :a => a, if:)\n  g(bar: bar, a: 1, if:)\nend\n"
    )
  end
end
