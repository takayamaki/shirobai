# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/RedundantFreeze`.
#
# The vendor spec covers the canonical immutable-literal cases, the frozen
# string literal magic-comment matrix, and the regexp/range version gate. A
# number of structural quirks were uncovered by probing stock rubocop directly
# and are pinned here because corpus parity is disposable and because the AST
# shape the cop relies on differs between parser-gem and prism:
#
# - **`immutable_literal?` truth table**: rubocop-ast's `IMMUTABLE_LITERALS`
#   (`LITERALS - MUTABLE_LITERALS`) includes `dsym` (interpolated symbol),
#   `complex` (prism `ImaginaryNode`), and `rational` (prism `RationalNode`).
#   A `dsym` is immutable even though it interpolates. Pinned because a naive
#   "only plain literals" reading would drop these.
# - **`strip_parenthesis` on a multi-statement begin**: stock takes
#   `children.first` of a paren `begin`, so `(1; 2).freeze` strips to the int
#   `1` and IS flagged, while `(a; b).freeze` strips to the send `a` and is
#   NOT. Empty parens `().freeze` and double parens `((1+1)).freeze` are not
#   flagged. In prism these are `ParenthesesNode` wrapping a `StatementsNode`.
# - **operation arms asymmetry**: arm 1 (`(send {float int} OP _)`) allows
#   `<<` and any argument; arm 2 (`(send !{(str _) array} OP {float int})`)
#   excludes `<<` and a string/array receiver but requires a numeric argument.
#   So `(1 << 2)` and `(1 + b)` flag (arm 1) but `(a << 2)` and `(a + b)` do
#   not; `(a + 1)` and `("x#{y}" + 1)` flag (arm 2) but `("x" + 1)` and
#   `([1] + 1)` do not.
# - **count/length/size arms**: the `(send _ {count length size} ...)` and
#   `(any_block (send ...) ...)` arms match a bare send OR a send-with-block
#   (block / numbered-parameter / `it` block), with or without a receiver, but
#   NOT a parenthesized call (`([1,2].count).freeze` is a `ParenthesesNode`)
#   and NOT a chain whose direct receiver is a different method
#   (`foo.count.bar.freeze`).
# - **no `on_csend`**: `x&.freeze` is never flagged.
# - **`.freeze` with a block**: `(1 + 2).freeze { }` still fires (RESTRICT_ON_SEND
#   is `:freeze`), and the offense highlight ends at the `freeze` selector, not
#   the block.
# - **frozen-string-literal enablement**: the string branch depends on the
#   leading `# frozen_string_literal:` comment (first specified value wins,
#   `true`/`false`/dash/emacs forms) with `AllCops/StringLiteralsFrozenByDefault`
#   as the fallback. Only leading comment lines count.
# - **target-version gating**: on `TargetRubyVersion >= 3.0` an interpolated
#   string is not a frozen candidate and a regexp/range literal is immutable;
#   on `2.7` the reverse holds.
RSpec.describe Shirobai::Cop::Style::RedundantFreeze do
  include EdgeCaseParity

  klasses = [
    RuboCop::Cop::Style::RedundantFreeze,
    Shirobai::Cop::Style::RedundantFreeze
  ]

  def build_config(target: 3.1, sfbd: :unset)
    hash = RuboCop::ConfigLoader.default_configuration.to_h.dup
    allcops = hash["AllCops"].dup
    allcops["TargetRubyVersion"] = target
    allcops["StringLiteralsFrozenByDefault"] = sfbd unless sfbd == :unset
    hash["AllCops"] = allcops
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(hash, "(test)"), "(test)"
    )
  end

  let(:config) { build_config }

  describe "immutable_literal? truth table" do
    %w[1 1.5 1r 1i 1ri :sym true false nil].each do |lit|
      it "flags `#{lit}.freeze`" do
        expect_autocorrect_parity(*klasses, "CONST = #{lit}.freeze\n", config)
      end
    end

    it "flags an interpolated symbol `:\"a\#{b}\".freeze` (dsym is immutable)" do
      expect_autocorrect_parity(*klasses, ":\"a\#{b}\".freeze\n", config)
    end
  end

  describe "strip_parenthesis quirks" do
    it "flags `(1; 2).freeze` (multi-statement begin strips to first int)" do
      expect_autocorrect_parity(*klasses, "(1; 2).freeze\n", config)
    end

    it "does NOT flag `(a; b).freeze` (first child is not immutable)" do
      expect_lint_parity(*klasses, "(a; b).freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "(a; b).freeze\n", config)).to be_empty
    end

    it "does NOT flag `().freeze` (empty parens)" do
      expect_lint_parity(*klasses, "().freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "().freeze\n", config)).to be_empty
    end

    it "does NOT flag `((1+1)).freeze` (double parens; inner child is a begin)" do
      expect_lint_parity(*klasses, "((1+1)).freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "((1+1)).freeze\n", config)).to be_empty
    end
  end

  describe "operation arms" do
    {
      "(1 + 2).freeze\n" => true,
      "(1 << 2).freeze\n" => true,       # arm 1 includes `<<`
      "(1 + b).freeze\n" => true,        # arm 1: numeric receiver, any arg
      "(a << 2).freeze\n" => false,      # arm 2 excludes `<<`
      "(a + 1).freeze\n" => true,        # arm 2: non-str/array recv, numeric arg
      "(\"x\#{y}\" + 1).freeze\n" => true, # arm 2: dstr recv is not `(str _)`
      "(\"x\" + 1).freeze\n" => false,   # arm 2 excludes str-literal receiver
      "([1] + 1).freeze\n" => false,     # arm 2 excludes array receiver
      "(a + b).freeze\n" => false,       # neither arm (non-numeric arg)
      "(a % b).freeze\n" => false,
      "(2 > 1).freeze\n" => true,        # arm 3 comparison
      "(a > b).freeze\n" => true,
      "('a' > 'b').freeze\n" => true,
      "('a' + 'b').freeze\n" => false,   # string concat is mutable
      "([42] * 42).freeze\n" => false
    }.each do |src, flagged|
      it "#{flagged ? "flags" : "does not flag"} `#{src.strip}`" do
        if flagged
          expect_autocorrect_parity(*klasses, src, config)
        else
          expect_lint_parity(*klasses, src, config, expect_offenses: false)
          expect(lint_offenses(klasses.first, src, config)).to be_empty
        end
      end
    end
  end

  describe "count / length / size arms" do
    [
      "[1, 2].count.freeze\n",
      "[1, 2].size.freeze\n",
      "x.length.freeze\n",
      "x.count { |e| e }.freeze\n",   # block
      "x.count { _1 }.freeze\n",      # numbered-parameter block
      "x.count { it }.freeze\n",      # `it` block
      "x.size(arg).freeze\n",         # arguments
      "count.freeze\n"                # receiverless
    ].each do |src|
      it "flags `#{src.strip}`" do
        expect_autocorrect_parity(*klasses, src, config)
      end
    end

    it "does NOT flag `([1, 2].count).freeze` (parenthesized call)" do
      expect_lint_parity(*klasses, "([1, 2].count).freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "([1, 2].count).freeze\n", config)).to be_empty
    end

    it "does NOT flag `foo.count.bar.freeze` (direct receiver is not count/size)" do
      expect_lint_parity(*klasses, "foo.count.bar.freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "foo.count.bar.freeze\n", config)).to be_empty
    end
  end

  describe "send-shape gating" do
    it "does NOT flag safe navigation `x&.freeze`" do
      expect_lint_parity(*klasses, "x&.freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "x&.freeze\n", config)).to be_empty
    end

    it "does NOT flag receiverless `freeze`" do
      expect_lint_parity(*klasses, "freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "freeze\n", config)).to be_empty
    end

    it "does NOT flag a method-call receiver `Something.new.freeze`" do
      expect_lint_parity(*klasses, "Something.new.freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "Something.new.freeze\n", config)).to be_empty
    end

    it "flags `.freeze` with a trailing block, highlight excluding the block" do
      expect_autocorrect_parity(*klasses, "(1 + 2).freeze { }\n", config)
    end
  end

  describe "frozen string literal enablement" do
    it "flags a string with a leading `# frozen_string_literal: true`" do
      expect_autocorrect_parity(*klasses, "# frozen_string_literal: true\n'str'.freeze\n", config)
    end

    it "does NOT flag a string without the magic comment (default config)" do
      expect_lint_parity(*klasses, "'str'.freeze\n", config, expect_offenses: false)
      expect(lint_offenses(klasses.first, "'str'.freeze\n", config)).to be_empty
    end

    it "flags an empty string when StringLiteralsFrozenByDefault is true" do
      cfg = build_config(sfbd: true)
      expect_autocorrect_parity(*klasses, "\"\".freeze\n", cfg)
    end

    it "lets a `# frozen_string_literal: false` comment win over sfbd true" do
      cfg = build_config(sfbd: true)
      expect_lint_parity(
        *klasses, "# frozen_string_literal: false\n\"\".freeze\n", cfg, expect_offenses: false
      )
      expect(
        lint_offenses(klasses.first, "# frozen_string_literal: false\n\"\".freeze\n", cfg)
      ).to be_empty
    end

    it "uses the FIRST fsl-specified leading comment" do
      cfg = build_config(sfbd: true)
      src = "# frozen_string_literal: false\n# frozen_string_literal: true\n\"\".freeze\n"
      expect_lint_parity(*klasses, src, cfg, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, cfg)).to be_empty
    end

    it "accepts the dash form `# frozen-string-literal: true`" do
      expect_autocorrect_parity(*klasses, "# frozen-string-literal: true\n'str'.freeze\n", config)
    end

    it "accepts the emacs form" do
      expect_autocorrect_parity(
        *klasses, "# -*- frozen_string_literal: true -*-\n'str'.freeze\n", config
      )
    end

    it "ignores a magic comment that is not leading" do
      src = "x = 1\n# frozen_string_literal: true\n'str'.freeze\n"
      expect_lint_parity(*klasses, src, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, config)).to be_empty
    end

    it "flags a plain heredoc but not an interpolated one (sfbd true)" do
      cfg = build_config(sfbd: true)
      expect_autocorrect_parity(*klasses, "<<~HD.freeze\n  plain\nHD\n", cfg)
      expect_lint_parity(*klasses, "<<~HD.freeze\n  a\#{x}\nHD\n", cfg, expect_offenses: false)
    end

    it "treats adjacent string literals as an uninterpolated candidate (sfbd true)" do
      cfg = build_config(sfbd: true)
      expect_autocorrect_parity(*klasses, "\"a\" \"b\".freeze\n", cfg)
    end
  end

  describe "target-version gating" do
    it "flags a regexp on 3.0+ but not on 2.7" do
      expect_autocorrect_parity(*klasses, "/re/.freeze\n", build_config(target: 3.1))
      expect_lint_parity(*klasses, "/re/.freeze\n", build_config(target: 2.7), expect_offenses: false)
      expect(lint_offenses(klasses.first, "/re/.freeze\n", build_config(target: 2.7))).to be_empty
    end

    it "flags a range on 3.0+ but not on 2.7" do
      expect_autocorrect_parity(*klasses, "(1..2).freeze\n", build_config(target: 3.1))
      expect_lint_parity(
        *klasses, "(1..2).freeze\n", build_config(target: 2.7), expect_offenses: false
      )
      expect(lint_offenses(klasses.first, "(1..2).freeze\n", build_config(target: 2.7))).to be_empty
    end

    it "flags an interpolated string on 2.7 (with fsl) but not on 3.0+" do
      src = "# frozen_string_literal: true\n\"a\#{x}\".freeze\n"
      expect_autocorrect_parity(*klasses, src, build_config(target: 2.7))
      expect_lint_parity(*klasses, src, build_config(target: 3.1), expect_offenses: false)
      expect(lint_offenses(klasses.first, src, build_config(target: 3.1))).to be_empty
    end
  end

  describe "multibyte source" do
    it "flags `(あ > い).freeze` with byte-correct ranges" do
      expect_autocorrect_parity(*klasses, "(あ > い).freeze\n", config)
    end

    it "flags a multibyte symbol `:あ.freeze`" do
      expect_autocorrect_parity(*klasses, "CONST = :あ.freeze\n", config)
    end
  end
end
