# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Style/MutableConstant`.
#
# Detection (`on_casgn`), the `shareable_constant_value` scope scan (which reads
# `processed_source.lines`, not tokens), and the `.freeze` autocorrect (splat
# expansion, array bracketing, range / send parenthesizing, recursive nested
# freezing) all run stock's own code verbatim on the parser AST, so they match
# stock by construction. The ONLY shirobai-specific part is the token-free
# `frozen_string_literals_enabled?` Rust scan. These differential cases pin that
# scan through comment-prefix shapes the vendor spec under-tests (a shebang
# before the magic comment, a frozen comment after `__END__`, an explicit
# `false`, and the Ruby-3.0 interpolated-string carve-out), plus the wide
# autocorrect branches so a refactor cannot silently regress them.
RSpec.describe Shirobai::Cop::Style::MutableConstant do
  include EdgeCaseParity

  def mc_config(overrides = {})
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Style/MutableConstant" => { "Enabled" => true } }.merge(overrides), "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Style::MutableConstant,
    Shirobai::Cop::Style::MutableConstant
  ]

  describe "token-free frozen_string_literals_enabled? scan" do
    it "does not flag a plain string constant under a frozen comment after a shebang" do
      src = "#!/usr/bin/env ruby\n# frozen_string_literal: true\nCONST = 'str'\n"
      expect_lint_parity(*klasses, src, mc_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, mc_config)).to be_empty
    end

    it "flags a plain string constant under `frozen_string_literal: false`" do
      expect_autocorrect_parity(*klasses, "# frozen_string_literal: false\nCONST = 'str'\n", mc_config)
    end

    it "flags a plain string constant with no magic comment" do
      expect_autocorrect_parity(*klasses, "CONST = 'str'\n", mc_config)
    end

    it "does not treat a frozen comment after __END__ as leading" do
      src = "CONST = 'str'\n__END__\n# frozen_string_literal: true\n"
      expect_autocorrect_parity(*klasses, src, mc_config)
    end

    it "still flags an interpolated string under a frozen comment (Ruby 3.0 carve-out)" do
      # From Ruby 3.0 an interpolated string is not frozen by the magic comment,
      # so it must still be frozen explicitly.
      expect_autocorrect_parity(*klasses, "# frozen_string_literal: true\nX = 1\nCONST = \"a\#{X}b\"\n", mc_config)
    end
  end

  describe "shareable_constant_value magic comment (line scan, not tokens)" do
    it "suppresses the offense when the directive is in scope" do
      src = "# shareable_constant_value: literal\nCONST = [1, 2, 3]\n"
      expect_lint_parity(*klasses, src, mc_config, expect_offenses: false)
      expect(lint_offenses(klasses.first, src, mc_config)).to be_empty
    end

    it "still flags when the directive value is none" do
      expect_autocorrect_parity(*klasses, "# shareable_constant_value: none\nCONST = [1, 2, 3]\n", mc_config)
    end
  end

  describe "autocorrect branches (stock on the parser AST)" do
    it "expands a splat and appends freeze" do
      expect_autocorrect_parity(*klasses, "FOO = *[1, 2, 3]\n", mc_config)
    end

    it "brackets a bare array before freezing" do
      expect_autocorrect_parity(*klasses, "CONST = 1, 2, 3\n", mc_config)
    end

    it "parenthesizes a dotless send before freezing in strict mode" do
      expect_autocorrect_parity(*klasses, "CONST = foo\n", mc_config("Style/MutableConstant" => { "EnforcedStyle" => "strict" }))
    end

    it "recursively freezes nested literals" do
      expect_autocorrect_parity(
        *klasses,
        "CONST = [{ a: [], b: 'foo' }]\n",
        mc_config("Style/MutableConstant" => { "Recursive" => true })
      )
    end
  end
end
