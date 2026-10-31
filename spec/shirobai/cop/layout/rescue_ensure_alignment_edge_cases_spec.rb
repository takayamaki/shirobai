# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/RescueEnsureAlignment`.
#
# Detection (`on_resbody` / `on_ensure`), the `alignment_node` resolution
# (access modifier, assignment, block-send), the offense, and the whitespace
# autocorrect all run stock's own code verbatim on the parser AST, so they match
# stock by construction. The ONLY shirobai-specific part is the replacement of
# stock's token-scan `on_new_investigation` (which builds `@modifier_locations`)
# with the prism `RescueModifierNode` keyword set. These differential cases pin
# that the modifier set is exact — a modifier rescue is SKIPPED while a real
# begin/def/block rescue on the same file is still checked — across the
# alignment-node variants and both `BeginEndAlignment` styles.
RSpec.describe Shirobai::Cop::Layout::RescueEnsureAlignment do
  include EdgeCaseParity

  def rea_config(begin_end_style = "start_of_line")
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        {
          "Layout/RescueEnsureAlignment" => { "Enabled" => true },
          "Layout/BeginEndAlignment" => {
            "Enabled" => true, "EnforcedStyleAlignWith" => begin_end_style
          }
        },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Layout::RescueEnsureAlignment,
    Shirobai::Cop::Layout::RescueEnsureAlignment
  ]

  describe "modifier rescue is skipped (the toucher's whole purpose)" do
    it "does not flag a modifier rescue" do
      expect_lint_parity(*klasses, "def m\n  z = y rescue 0\nend\n", rea_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "def m\n  z = y rescue 0\nend\n", rea_config)).to be_empty
    end

    it "skips the modifier but still flags a real misaligned rescue on the same file" do
      src = "z = y rescue 0\nbegin\n  x\n    rescue\n  y\nend\n"
      expect_autocorrect_parity(*klasses, src, rea_config)
    end

    it "skips a modifier rescue nested inside a checked begin/rescue body" do
      src = "begin\n  a = b rescue 1\n    rescue\n  c\nend\n"
      expect_autocorrect_parity(*klasses, src, rea_config)
    end
  end

  describe "alignment_node variants (stock resolution)" do
    it "aligns to an access modifier (`private def`)" do
      src = "class A\n  private def m\n    x\n      rescue\n    y\n  end\nend\n"
      expect_autocorrect_parity(*klasses, src, rea_config)
    end

    it "aligns to the assignment start for `x = begin` (start_of_line)" do
      expect_autocorrect_parity(*klasses, "x = begin\n      foo\n  rescue\n      bar\n    end\n", rea_config)
    end

    it "aligns to the `begin` keyword for `x = begin` (keyword style)" do
      expect_autocorrect_parity(
        *klasses,
        "x = begin\n      foo\n  rescue\n      bar\n    end\n",
        rea_config("keyword")
      )
    end

    it "aligns a block rescue to the start of a multiline receiver (keyword style)" do
      src = "foo.bar(1)\n   .baz do\n  x\n      rescue\n  y\nend\n"
      expect_autocorrect_parity(*klasses, src, rea_config("keyword"))
    end
  end

  describe "inline expression before the keyword" do
    it "reports but does not autocorrect when non-blank source precedes the rescue" do
      # stock's `autocorrect` returns nil when the whitespace range is not blank,
      # so the offense is correctable-without-correction. Both sides must agree.
      src = "begin\n  something\n  x = 1; rescue\n  y\nend\n"
      expect_lint_parity(*klasses, src, rea_config)
    end
  end
end

# 1.92 tabs autocorrect (rubocop#15769 / #15837): under
# `Layout/IndentationStyle: tabs` the keyword takes the target line's own
# indentation, and is left alone when the target does not begin its line.
# Mixed tab/space anchors and a nested `def` are not in the vendor spec.
RSpec.describe Shirobai::Cop::Layout::RescueEnsureAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::RescueEnsureAlignment,
    Shirobai::Cop::Layout::RescueEnsureAlignment
  ]

  it "matches stock under tabs" do
    cfg = RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/IndentationStyle" => { "EnforcedStyle" => "tabs" } }, "(test)"),
      "(test)"
    )
    source = "\tbegin\n\t\tfoo\n rescue\n\t\tbar\n\tend\n" \
             "\t  def m\n\t\tfoo\n\t\t\tensure\n\t\tbar\n\t  end\n" \
             "\tx = [begin\n\t\tfoo\n  rescue\n\t\tbar\n\tend]\n"
    expect_lint_parity(*klasses, source, cfg)
    expect_autocorrect_parity(*klasses, source, cfg)
  end
end

# 1.92.2 (rubocop#16183): `alignment_source` takes `node.lhs.source_range`
# for `op_asgn` / `or_asgn` / `and_asgn` like `masgn`. Before, an index or
# attribute target fell to the receiver branch and crashed on
# `Parser::Source::Map::Send#name`. The message text is built from this
# source, so every operator-assignment flavor is pinned with both
# `BeginEndAlignment` settings.
RSpec.describe Shirobai::Cop::Layout::RescueEnsureAlignment do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::RescueEnsureAlignment,
    Shirobai::Cop::Layout::RescueEnsureAlignment
  ]

  def begin_end_config(begin_end)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/BeginEndAlignment" => begin_end }, "(test)"),
      "(test)"
    )
  end

  sources = {
    "index or-assignment" => "h[:k] ||= foo do\n  bar\n  rescue E\n  baz\nend\n",
    "index and-assignment" => "h[:k] &&= foo do\n  rescue\nend\n",
    "index operator assignment" => "h[:k] += foo do\n    ensure\nend\n",
    "attribute or-assignment" => "obj.attr ||= foo do\n  ensure\nend\n",
    "safe-navigation attribute assignment" => "obj&.attr ||= foo do\n  rescue\nend\n",
    "local operator assignment" => "x += foo do\n  rescue\nend\n",
    "instance variable or-assignment" => "@a ||= foo do\n  rescue\nend\n",
    "constant path or-assignment" => "Foo::BAR ||= foo do\n  rescue\nend\n",
    "begin block on an index or-assignment" => "h[:k] ||= begin\n    foo\n  rescue\n    bar\n  end\n",
    "multi-line index target" => "h[\n  :k\n] ||= foo do\n  rescue\nend\n",
    "non-ASCII target" => "変数[:キー] ||= foo do\n  rescue\nend\n"
  }

  sources.each do |name, source|
    [{ "Enabled" => false }, { "Enabled" => true, "EnforcedStyleAlignWith" => "start_of_line" },
     { "Enabled" => true, "EnforcedStyleAlignWith" => "begin" }].each do |begin_end|
      it "matches stock for a #{name} (BeginEndAlignment #{begin_end})" do
        cfg = begin_end_config(begin_end)
        expect_lint_parity(*klasses, source, cfg)
        expect_correction_edits_parity(*klasses, source, cfg)
        expect_autocorrect_parity(*klasses, source, cfg)
      end
    end
  end
end
