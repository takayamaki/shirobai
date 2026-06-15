# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/AmbiguousBlockAssociation`.
#
# The vendor spec exercises the canonical positive / negative paths but does
# NOT pin a number of structural quirks the stock real-machine probe surfaced.
# Corpus parity is disposable (clean corpora trigger few of these patterns), so
# this spec is the only durable guard for the AST shape differences between
# parser-gem and prism that this cop is sensitive to:
#
#   - **parser `:block` is a single node kind, prism splits it into
#     `BlockNode` and `LambdaNode`**. Stock's `last_argument.any_block_type?`
#     covers BOTH (`lambda { … }` / `proc { … }` / `Proc.new { … }` and
#     `-> { … }` all surface as parser `:block`). prism splits the lambda
#     literal into `LambdaNode`. The wrapper has to gate on:
#       (a) `CallNode` with `block: Some(BlockNode | LambdaNode)` AND
#           sender name NOT `:lambda` / `:proc` / `Proc.new`, OR
#       (b) the last_argument is a `LambdaNode` directly (also excluded by
#           stock's `lambda_or_proc?` early return).
#
#   - **`block_pass` (`&block`) lives on `CallNode.block` in prism**, not in
#     `arguments`. A `foo(&blk)` last argument never trips `any_block_type?`.
#
#   - **`Hash[some_method a { … }]` (`:[]` outer)**: outer `Hash.[]` last_arg
#     is `CallNode(some_method)` whose own `.block()` is None — outer is NOT
#     flagged. INNER `some_method a { … }` IS flagged (visitor recurses).
#
#   - **`foo[bar { … }]` (`:[]` outer)**: outer name IS `:[]` (excluded by
#     `node.method?(:[])`).
#
#   - **outer `assignment?` (setter, `loc.operator == :=`)**: prism's
#     `equal_loc.is_some()`. Setter call `obj.foo= bar { … }` is excluded.
#
#   - **outer `operator_method?`**: `foo == bar { baz a }` outer is `==`,
#     in `OPERATOR_METHODS`.
#
#   - **AllowedPatterns regexp**: when the cop config carries `Regexp`
#     entries the bundle path is skipped (`bundle_eligible?` is false) and
#     the standalone per-cop entry is used. Pinned so a regression in
#     `bundle_eligible?` would be caught.
#
#   - **Inner sender `lambda` / `proc` / `Proc.new`**: stock's
#     `lambda_or_proc?` (`(any_block (send nil :lambda) …)` /
#     `{(block (send nil :proc) …) (block (send Proc :new) …) (send Proc :new)}`)
#     excludes these. prism mirrors via the inner CallNode's
#     `name == :lambda|:proc` (no receiver) AND `name == :new` with
#     receiver `Proc`.
#
#   - **autocorrect range boundaries**: stock's
#     `node.loc.selector.end.join(node.first_argument.source_range.begin)`.
#     Selector in prism is `message_loc()`; first_argument's begin is
#     `args.arguments[0].location().start_offset()`. Predicate methods (`is?`)
#     include `?` in the selector.
#
#   - **csend (`&.`) outer**: same logic as `.`; pinned to lock the
#     `on_csend` alias path through `visit_call_node`.
#
#   - **MSG `%<method>s` is the INNER SENDER SOURCE**, not the inner
#     block-bearing CallNode source. For `change { order.events }` the
#     method substitution is `change`, not `change { order.events }`. The
#     wrapper reads `inner_send_start..inner_send_end` which strips the
#     `block.location.start` and any trailing whitespace, exactly mirroring
#     parser-gem's `(block (send …) …)` `.send_node.source`.
RSpec.describe Shirobai::Cop::Lint::AmbiguousBlockAssociation do
  include EdgeCaseParity

  let(:config) { RuboCop::ConfigLoader.default_configuration }

  klasses = [
    RuboCop::Cop::Lint::AmbiguousBlockAssociation,
    Shirobai::Cop::Lint::AmbiguousBlockAssociation
  ]

  describe "last_argument shape gating" do
    it "does NOT flag when the last argument is a `&block` block-pass" do
      # block_pass lives on `CallNode.block` in prism, NOT in the arguments
      # list, so `last_argument` is never the block_pass and the guard
      # short-circuits before any block check.
      src = "foo bar(&blk), x\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "does NOT flag when the last argument is a `LambdaNode` (lambda literal)" do
      # `foo ->(a) { bar a }` — parser surfaces this as a `(block (send nil
      # :lambda) …)` and stock's `lambda_or_proc?` excludes it. prism gives
      # `LambdaNode` directly; the wrapper bails on that shape too.
      expect_lint_parity(*klasses, "foo ->(a) { bar a }\n", config, expect_offenses: false)
    end

    it "does NOT flag a `do…end` block (the block wraps the OUTER call, not the inner)" do
      # `some_method a do …; end` parses as `(block (send nil :some_method
      # (send nil :a)) …)` — the OUTER `some_method` takes the block, not
      # `a`. last_argument is the bare send `a`, which is not block-bearing.
      expect_lint_parity(*klasses, "some_method a do;puts 'x';end\n", config, expect_offenses: false)
    end

    it "does NOT flag when the inner sender takes arguments (`fetch(:a) { … }`)" do
      src = "env ENV.fetch(\"ENV\") { \"dev\" }\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
    end

    it "flags an inner CallNode whose receiver is a method chain" do
      # `assert_equal posts.find { |p| p.title == "Foo" }, results.first` is
      # accepted by stock (last_argument is `results.first`, not the block).
      # But the variant where the block-bearing call is LAST is flagged.
      # The vendor spec doesn't test a chained-receiver inner-sender as the
      # last argument explicitly; pin it.
      src = "some_method posts.find { |p| p.title == \"Foo\" }\n"
      expect_lint_parity(*klasses, src, config)
    end
  end

  describe "outer call method name gating" do
    it "does NOT flag when the outer method is `==`" do
      expect_lint_parity(*klasses, "foo == bar { baz a }\n", config, expect_offenses: false)
    end

    it "does NOT flag when the outer method is `[]` (`Hash[...]` form)" do
      # OUTER is `Hash[…]` (`:[]`). The INNER `some_method a { … }` IS
      # flagged — pinned at column 5.
      stock = expect_lint_parity(*klasses, "Hash[some_method a { |el| el }]\n", config)
      expect(stock.first[0]).to eq(5)
    end

    it "does NOT flag when the outer method is `[]` (`foo[bar { … }]` form)" do
      # OUTER is `foo[…]` (`:[]`) AND its last_arg IS a block-bearing call;
      # excluded by the `:[]` name check.
      expect_lint_parity(*klasses, "foo[bar { |a| a }]\n", config, expect_offenses: false)
    end

    it "does NOT flag when the outer is a setter (`obj.attr= a { … }`)" do
      # `obj.attr = a { … }` is a setter call: prism `equal_loc.is_some()`.
      # stock's `assignment?` (= `setter_method?`) excludes.
      expect_lint_parity(*klasses, "obj.attr = a { |el| el }\n", config, expect_offenses: false)
    end
  end

  describe "inner sender lambda / proc / Proc.new" do
    it "does NOT flag `foo lambda { … }`" do
      expect_lint_parity(*klasses, "scope :active, lambda { do_it }\n", config, expect_offenses: false)
    end

    it "does NOT flag `foo proc { … }`" do
      expect_lint_parity(*klasses, "scope :active, proc { do_it }\n", config, expect_offenses: false)
    end

    it "does NOT flag `foo Proc.new { … }`" do
      # rubocop-ast `proc?` includes `(block (send Proc :new) …)`. prism:
      # inner CallNode receiver = ConstantReadNode(`Proc`), name = `:new`.
      expect_lint_parity(*klasses, "scope :active, Proc.new { do_it }\n", config, expect_offenses: false)
    end
  end

  describe "csend (`&.`) outer" do
    it "flags `Foo&.some_method a { … }`" do
      expect_lint_parity(*klasses, "Foo&.some_method a { |el| el }\n", config)
    end
  end

  describe "autocorrect range boundaries" do
    it "wraps the `(` immediately after the selector for a plain selector" do
      # `some_method a { |el| el }` — selector `some_method` ends at 11,
      # first arg `a` starts at 12. Autocorrect: remove the single space at
      # `[11, 12)` then insert `(` and `)` after the last arg.
      expect(expect_autocorrect_parity(*klasses, "some_method a { |el| el }\n", config))
        .to eq("some_method(a { |el| el })\n")
    end

    it "wraps the `(` after the receiver-`.` chain selector" do
      expect(expect_autocorrect_parity(*klasses, "Foo.some_method a { |el| el }\n", config))
        .to eq("Foo.some_method(a { |el| el })\n")
    end

    it "wraps the `(` after the csend (`&.`) selector" do
      expect(expect_autocorrect_parity(*klasses, "Foo&.some_method a { |el| el }\n", config))
        .to eq("Foo&.some_method(a { |el| el })\n")
    end
  end

  describe "MSG `%<method>s` substitution" do
    it "uses the INNER SENDER source (stripped of the block), not the whole block-bearing call" do
      # The substituted method must be `change`, not
      # `change { order.events }`. The wrapper's `inner_send_end` strips the
      # block-leading whitespace, mirroring parser-gem's `(block (send …) …)`
      # `.send_node.source` exactly.
      src = "expect { order.expire }.to change { order.events }\n"
      stock = expect_lint_parity(*klasses, src, config)
      msg = stock.first[2]
      expect(msg).to include("the `change` method call")
      expect(msg).not_to include("the `change { order.events }` method call")
    end

    it "uses the inner sender with its receiver chain when the inner has a chain" do
      # `expect(order).to receive(:complete).twice { OrderCount.update! }` —
      # the inner sender is `receive(:complete).twice`. Pinned so a refactor
      # of the chain-end byte computation cannot regress the substitution.
      src = "expect(order).to receive(:complete).twice { OrderCount.update! }\n"
      cfg = make_config("AllowedPatterns" => [/receive\(.*?\)\.twice/])
      # With the pattern allowed, no offense fires; but the pattern matches
      # the inner-sender source so we use a non-matching pattern instead.
      cfg_default = config
      stock = expect_lint_parity(*klasses, src, cfg_default)
      msg = stock.first[2]
      expect(msg).to include("`receive(:complete).twice`")
      _ = cfg
    end
  end

  describe "AllowedPatterns config (standalone fallback path)" do
    it "skips when an AllowedPatterns regexp matches the inner sender source" do
      cfg = make_config("AllowedPatterns" => [/change/])
      expect_lint_parity(
        *klasses,
        "expect { order.expire }.to change { order.events }\n",
        cfg,
        expect_offenses: false
      )
    end

    it "skips when a regexp matches a chained inner sender like `receive(:c).twice`" do
      cfg = make_config("AllowedPatterns" => [/receive\(.*?\)\.twice/])
      expect_lint_parity(
        *klasses,
        "expect(order).to receive(:complete).twice { OrderCount.update! }\n",
        cfg,
        expect_offenses: false
      )
    end

    it "still flags an inner sender whose source does NOT match any pattern" do
      cfg = make_config("AllowedPatterns" => [/change/])
      stock = expect_lint_parity(
        *klasses,
        "expect { order.expire }.to update { order.events }\n",
        cfg
      )
      expect(stock.first[2]).to include("`update`")
    end
  end

  def make_config(overrides)
    # `Config#to_h` is a SHALLOW copy: nested per-cop hashes still share
    # identity with the default_configuration. Mutating an inner hash here
    # would silently leak into every later spec that asks for the default
    # configuration (seen first when `AllowedPatterns: [/change/]` made the
    # vendor spec's `expect { ... }.to change { ... }` fixture fall through
    # the regexp filter and report zero offenses). `merge` returns a fresh
    # hash so the default stays untouched.
    default = RuboCop::ConfigLoader.default_configuration
    hash = default.to_h
    cop_hash = (hash["Lint/AmbiguousBlockAssociation"] || {}).merge(overrides)
    hash = hash.merge("Lint/AmbiguousBlockAssociation" => cop_hash)
    RuboCop::Config.new(hash, default.loaded_path)
  end
end
