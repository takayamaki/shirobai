# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard: clobber confinement keeps hash groups separate.
#
# When adjacent key/value corrections inside ONE hash collide
# (`ClobberingError`), stock drops the remaining offenses of THAT hash but
# leaves every OTHER hash intact. shirobai reproduces this by handing a hash
# group id from Rust and confining the ClobberingError inside the group. If the
# group boundary collapses, a clobber in one hash would swallow another hash's
# offenses (or resurrect them), a divergence that is invisible to count parity
# when it nets out.
#
# Corpus-only before this spec (it was first seen on a specific corpus file;
# the vendor + non_ascii fixtures only ever use a SINGLE hash). These cases pin
# that two independently-misaligned hashes each keep their own offense and are
# each corrected, with stock and shirobai agreeing throughout.
RSpec.describe Shirobai::Cop::Layout::HashAlignment do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Layout::HashAlignment,
    Shirobai::Cop::Layout::HashAlignment
  ]

  # Two separate multiline hashes, each with its own misaligned key. Each hash
  # is its own correction group: both offenses survive and both are corrected.
  it "confines each hash's offenses to its own group (two misaligned hashes)" do
    source = "a = {\n  x: 1,\n   yy: 2\n}\nb = {\n  p: 1,\n   qq: 2\n}\n"
    stock = expect_lint_parity(*klasses, source, config)
    # Both hashes must contribute an offense (not just one) — guards that the
    # group split does not collapse the two hashes into one swallowed group.
    expect(stock.size).to eq(2)
    expect_autocorrect_parity(*klasses, source, config)
  end

  # Control: one hash misaligned, one already aligned. The aligned hash must
  # stay untouched (no spurious offense leaking across the group boundary).
  it "leaves an already-aligned neighbouring hash untouched" do
    source = "a = {\n  x: 1,\n   yy: 2\n}\nb = {\n  p: 1,\n  qq: 2\n}\n"
    stock = expect_lint_parity(*klasses, source, config)
    expect(stock.size).to eq(1)
    expect_autocorrect_parity(*klasses, source, config)
  end
  # 1.92 (rubocop#15717 / #15751 / #15821): a style that meets a key it cannot
  # right-align (key delta past the line start) drops out of the min_by
  # unless every style does; separator values go one space after the first
  # separator; a multiline key is left alone by the separator style and does
  # not shift its own separator under the table style. Multi-style configs
  # mix these with the key style, which the vendor spec does not cover.
  def ha_config(colon, rocket)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/HashAlignment" => { "EnforcedColonStyle" => colon, "EnforcedHashRocketStyle" => rocket } },
        "(test)"
      ),
      "(test)"
    )
  end

  source_192 = <<~SRC
    f(
      short: a,
        considerably_longer_key: b,
      c:   1
    )
    h = {
      a =>  1,
       bbbbbbbb => 2,
      c=>3
    }
    g = {
      [
        :x,
       :y] => 1,
         foo => 2,
      bar  =>   3
    }
    k = {
        a => 1,
      bb =>  2,
      cccccccc => 3,
      **opts,
    }
    m = {
      a: 0,
      bbbb: 1,
       c: 2
    }
  SRC

  [
    ["separator", "separator"],
    [%w[separator key], %w[separator table]],
    [%w[key separator], %w[table separator]],
    ["table", "table"]
  ].each do |colon, rocket|
    it "matches stock on unalignable keys and multiline keys (colon: #{colon}, rocket: #{rocket})" do
      cfg = ha_config(colon, rocket)
      expect_lint_parity(*klasses, source_192, cfg)
      expect_autocorrect_parity(*klasses, source_192, cfg)
    end
  end

  # 1.92.2 (rubocop#16134 / rubocop#16175): the value styles (table and
  # separator) cannot check a hash where a pair starts on the line of the
  # previous child (usually a `**kwsplat`), and they compute no delta for a
  # pair that does not begin its line. With several styles, deltas are still
  # computed for a hash one style cannot check, so every mix is pinned here,
  # down to the edit list.
  sources_1922 = {
    "pairs after kwsplats" => "x = { **a, b:  1,\n      **c, dd: 2 }\n",
    "rocket pairs after kwsplats" => "x = { **a, :b  => 1,\n      **c, :dd => 2 }\n",
    "a pair after a pair" => "h = {\n  a:   1,\n  bb:  2,\n  ccc: 3,\n  e: 5, f: 6\n}\n",
    "a kwsplat after a pair" => "h = {\n  a:  1,\n  bbb: 2, **c,\n  d: 3\n}\n",
    "call arguments" => "foo(**a, b:  1,\n    **c, dd: 2)\n",
    "non-ASCII keys" => "h = {\n  あ:   1,\n  いいい: 2,\n  e: 5, f: 6\n}\n"
  }
  styles_1922 = [
    %w[key key], %w[table table], %w[separator separator],
    [%w[key table], %w[key table]], [%w[table key], %w[table key]],
    [%w[separator key], %w[separator key]], [%w[key separator], %w[key separator]]
  ]

  sources_1922.each do |name, source|
    styles_1922.each do |colon, rocket|
      it "matches stock with #{name} (colon: #{colon}, rocket: #{rocket})" do
        cfg = ha_config(colon, rocket)
        expect_lint_parity(*klasses, source, cfg, expect_offenses: false)
        expect(correction_edits(klasses.last, source, cfg)).to eq(correction_edits(klasses.first, source, cfg))
        expect_autocorrect_parity(*klasses, source, cfg)
      end
    end
  end

  it "moves only the kwsplat when pairs follow kwsplats on the same line (key and table)" do
    cfg = ha_config(%w[key table], %w[key table])
    source = sources_1922["pairs after kwsplats"]
    expect_lint_parity(*klasses, source, cfg)
    expect_correction_edits_parity(*klasses, source, cfg)
    expect(expect_autocorrect_parity(*klasses, source, cfg))
      .to eq("x = { **a, b:  1,\n           **c, dd: 2 }\n")
  end

  it "corrects a misaligned pair that has another pair after it (table and key, non-ASCII)" do
    # `max_key_width` counts the chars of a key, not its bytes.
    cfg = ha_config(%w[table key], %w[table key])
    source = "h = {\n  あ:   1,\n  いい:  2,\n  ううう: 3,\n  e: 5, f: 6\n}\n"
    expect_lint_parity(*klasses, source, cfg)
    expect_correction_edits_parity(*klasses, source, cfg)
    expect_autocorrect_parity(*klasses, source, cfg)
  end
end
