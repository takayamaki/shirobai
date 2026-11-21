# frozen_string_literal: true

require "spec_helper"

# The `empty_lines` insert arm of `EmptyLineCorrector` (rubocop#16138, 1.92.2):
# stock now replaces the newline before the offense line with `"\n\n"`
# instead of inserting a `"\n"` before it. The final bytes are the same, so
# only the edit list (`Offense#corrections`) tells the two apart.
RSpec.describe Shirobai::Cop::Layout::EmptyLinesAroundClassBody do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::EmptyLinesAroundClassBody,
    Shirobai::Cop::Layout::EmptyLinesAroundClassBody
  ]

  def body_config(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Layout/EmptyLinesAroundClassBody" => { "EnforcedStyle" => style } }, "(test)"),
      "(test)"
    )
  end

  it "builds the same edits as stock for a missing blank line at both ends" do
    source = "class Foo\n  bar\nend\n"
    config = body_config("empty_lines")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "builds the same edits as stock for the empty_lines_special style" do
    source = "class Foo\n  bar\n\n  def baz; end\nend\n"
    config = body_config("empty_lines_special")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end

  it "builds the same edits as stock after a non-ASCII line and with CRLF" do
    source = "class Foo\r\n  \"ほげ\"\r\n  bar\r\nend\r\n"
    config = body_config("empty_lines")
    expect_lint_parity(*klasses, source, config)
    expect_correction_edits_parity(*klasses, source, config)
    expect_autocorrect_parity(*klasses, source, config)
  end
  # 1.92.3 mixin changes, per style:
  # - rubocop#16305: no blank line is required at an end the body shares
  #   with the class line (or the `end` line);
  # - rubocop#16309: `private if cond` / `private unless cond` counts as an
  #   access modifier for `empty_lines_special` (deferred message type `if`);
  # - rubocop#16372: no deferred blank line before a definition that shares
  #   a line with the previous statement (parser `last_line` of the sibling).
  {
    "empty_lines" => [
      "class Foo; bar\n  baz\nend\n",
      "class Foo\n  bar\n  baz end\n",
      "class Foo < Bar; baz\nend\n",
      "class Foo <\n  Bar; baz\nend\n",
      "class Foo\n  a\nrescue\n  b end\n",
      "class Foo\n  a\nensure end\n",
      "class << self; a\n  b\nend\n"
    ],
    "empty_lines_except_namespace" => [
      "class Foo; bar\nend\n",
      "class Foo; class Bar; end\nend\n"
    ],
    "beginning_only" => ["class Foo; bar\n\nend\n"],
    "ending_only" => ["class Foo\n\n  bar end\n"],
    "empty_lines_special" => [
      "class Foo; def a; end\nend\n",
      "class Foo\n  def a; end end\n",
      "class Foo\n  bar\n  public unless x\nend\n",
      "class Foo\n  bar\n  # c\n  protected if x\n\nend\n",
      "class Foo\n\n  private if x\n  def a; end\n\nend\n",
      "class Foo\n  private() if x\n\nend\n",
      "class Foo\n  private :a if x\n\nend\n",
      "class Foo\n  x ? private : nil\n\nend\n",
      "class Foo\n  if x then private end\n\nend\n",
      "class Foo\n  (private if x)\n\nend\n",
      "class Foo\n  (def a; end)\n\nend\n",
      "class Foo\n  (bar; def a; end)\n\nend\n",
      "class Foo\n  ()\n\nend\n",
      "class Foo\n  bar(\n    1); def a; end\n\nend\n",
      "class Foo\n  bar(\n    1)\n  def a; end\n\nend\n",
      "class Foo\n  bar; baz; def a; end\n\nend\n",
      "class Foo\n  x = <<~A; def a; end\n    t\n  A\n\nend\n",
      "class Foo\n  bar\n  private; def a; end\n\nend\n"
    ]
  }.each do |style, sources|
    sources.each do |src|
      it "matches stock under #{style} for #{src.inspect}" do
        config = body_config(style)
        expect_lint_parity(*klasses, src, config, expect_offenses: false)
        expect_autocorrect_parity(*klasses, src, config)
      end
    end
  end
end
