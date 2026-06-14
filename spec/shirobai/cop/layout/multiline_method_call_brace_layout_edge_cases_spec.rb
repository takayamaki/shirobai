# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/MultilineMethodCallBraceLayout`.
#
# The vendor spec covers `on_send` / `on_csend` and the three EnforcedStyles
# end-to-end, but several behaviours the implementation depends on are NOT
# pinned by the vendor spec and were discovered during stock probing:
#
#   - super / yield / def parens are NOT targets (only send/csend).
#   - assignment forms (`obj.attr = (...)`) are not send and therefore not
#     targets.
#   - the chained-call shape `foo(...).bar` shares its source-range begin_pos
#     with the inner call, so the wrapper has to disambiguate by (begin, end)
#     pair when relocating the parser-gem node for autocorrect — otherwise
#     pre-order `each_node` returns the OUTER send which has no own
#     `loc.begin`/`loc.end` and crashes the corrector.
#   - `last_line_heredoc?` pins its `parent` to the FIRST argument the helper
#     receives (`parent ||= node`), so any heredoc as the last argument always
#     matches (heredoc_end.last_line >= its own last_line). The vendor "heredoc
#     ignores heredocs that could share a last line" spec exercises this only
#     once on a same_line style; we pin it across styles to keep the wrapper
#     honest.
#   - `block_pass` (`&blk`) is the LAST argument from parser-gem's perspective
#     and stock treats it like any other element. Prism keeps block_pass on the
#     separate `block` field, which the rule re-splices.
#   - `lambda { ... }.call(...)` carries a multi-line receiver but the call's
#     own `(`/`)` may share a line — the `single_line_ignoring_receiver?`
#     override added by MMCBL on top of the shared mixin must short-circuit
#     before any style check.
RSpec.describe Shirobai::Cop::Layout::MultilineMethodCallBraceLayout do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Layout::MultilineMethodCallBraceLayout,
    Shirobai::Cop::Layout::MultilineMethodCallBraceLayout
  ]

  let(:default_config) { RuboCop::ConfigLoader.default_configuration }

  it "ignores super(...) — only send/csend dispatch fires" do
    src = "super(a,\n  b\n)\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "ignores yield(...) — only send/csend dispatch fires" do
    src = "yield(a,\n  b\n)\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "ignores def parens (the MethodDefinitionBraceLayout territory)" do
    src = "def foo(a,\n  b\n)\nend\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "treats block-pass (&blk) as the last argument when alone" do
    # Closing `)` on a line different from `&blk` triggers SAME_LINE_MESSAGE
    # under default (symmetrical) style.
    src = "foo(&blk\n)\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("foo(&blk)\n")
  end

  it "treats block-pass (&blk) as the last argument after others" do
    src = "foo(a,\n  &blk\n)\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("foo(a,\n  &blk)\n")
  end

  it "detects the INNER call in a chained shape foo(...).chained" do
    # Outer `.chained` send has no own `loc.begin`/`loc.end` for the cop, so
    # the rule must report the inner `foo(...)` brace. The wrapper resolves
    # the parser-gem node by (begin, end) pair so the corrector runs against
    # the inner send, not the outer chained send.
    src = "foo(a,\n  b\n).chained\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("foo(a,\n  b).chained\n")
  end

  it "detects the INNER call in a chained super(bar(...)) shape" do
    # super itself is out-of-scope; the inner `bar(...)` is the only target.
    src = "super(bar(baz,\n  ham\n))\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("super(bar(baz,\n  ham))\n")
  end

  it "ignores a heredoc last argument under symmetrical style too" do
    # Vendor's heredoc context uses same_line; here we pin symmetrical and
    # confirm `parent ||= node` still short-circuits the offense check.
    src = "foo(a,\n<<-EOM\nbaz\nEOM\n)\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "ignores a heredoc last argument under new_line style too" do
    new_line_config = config_with(default_config, "Layout/MultilineMethodCallBraceLayout",
                                  "EnforcedStyle" => "new_line")
    src = "foo(a,\n<<-EOM\nbaz\nEOM\n)\n"
    expect_lint_parity(*klasses, src, new_line_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, new_line_config)).to be_empty
  end

  it "relocates the chained method past `)` for a heredoc-arg call chain" do
    # Vendor spec line 33 covers this end-to-end, but the autocorrect text
    # involves three correctors (`)` insertion + chained-method delete + chained
    # source insert). Pin it as a regression so a refactor to the Rust→Ruby
    # interface cannot silently flip the autocorrect output.
    src = "foo(<<~EOS, arg\n  text\nEOS\n).do_something\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("foo(<<~EOS, arg).do_something\n  text\nEOS\n")
  end

  it "ignores a lambda receiver whose `(`/`)` happen to share a line" do
    # `lambda { x }.call(\n  1\n)` — the call's `(` is on line 1, `)` on
    # line 3, so single_line_ignoring_receiver? is false; closing brace IS
    # on a different line from the last arg → with opening on the same line
    # as the first arg, symmetrical style emits SAME_LINE. We pin the
    # negative case where `(` and `)` SHARE a line (no offense).
    src = "->(x) { x }.call(\n  1\n)\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "fires on a non-chained call wrapped in obj.method(...)" do
    # `self.foo(...)` is a send with a non-implicit receiver; the wrapper's
    # send-locator walks `(:send, :csend)` and must still find it.
    src = "self.foo(a,\n  b\n)\n"
    expect_lint_parity(*klasses, src, default_config)
    expect(expect_autocorrect_parity(*klasses, src, default_config))
      .to eq("self.foo(a,\n  b)\n")
  end

  it "ignores an unparenthesized (implicit) call" do
    src = "foo a,\n  b\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  it "ignores a multiline empty paren puts(\\n)" do
    src = "puts(\n)\n"
    expect_lint_parity(*klasses, src, default_config, expect_offenses: false)
    expect(lint_offenses(klasses.first, src, default_config)).to be_empty
  end

  def config_with(base, cop_name, overrides)
    hash = base.to_h.dup
    hash[cop_name] = (hash[cop_name] || {}).merge(overrides)
    RuboCop::Config.new(hash, base.loaded_path)
  end
end
