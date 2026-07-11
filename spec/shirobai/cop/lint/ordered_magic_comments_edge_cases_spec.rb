# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Lint/OrderedMagicComments`.
#
# Quirks probed against stock that the vendor spec does not pin:
#
# - the "other magic comment" bucket is `MagicComment#valid?` = the comment
#   `start_with?('#')` AND specifies a non-encoding magic kind. Beyond
#   `frozen_string_literal` (the only one the vendor spec exercises besides
#   `shareable_constant_value`), that includes `typed` and `rbs_inline`;
# - `rbs_inline` counts ONLY with an `enabled` / `disabled` value
#   (`rbs_inline_specified?` == `valid_rbs_inline_value?`), unlike the other
#   kinds where any TOKEN value is "specified";
# - the encoding bucket (`encoding_specified?`) has NO `start_with?('#')`
#   requirement, but the "other" bucket does: a leading-space
#   `  # frozen_string_literal: true` line is not a valid "other" comment, so
#   an encoding line after it has no partner;
# - stock overwrites `lines[1]` on every "other" line before the encoding line
#   is found, so a later encoding pairs with the LATEST preceding other line;
# - shebang and blank lines do not end the leading prefix, but a non-comment
#   token does (a magic-shaped hash literal further down is code, not a
#   leading comment).
RSpec.describe Shirobai::Cop::Lint::OrderedMagicComments do
  include EdgeCaseParity

  let(:config) do
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new({ "Lint/OrderedMagicComments" => { "Enabled" => true } }, "(test)"),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Lint::OrderedMagicComments,
    Shirobai::Cop::Lint::OrderedMagicComments
  ]

  describe "non-encoding magic kinds in the `other` bucket" do
    it "flags an encoding comment after a `typed` sigil" do
      expect_autocorrect_parity(*klasses, "# typed: true\n# encoding: ascii\n", config)
    end

    it "flags an encoding comment after an enabled `rbs_inline` comment" do
      expect_autocorrect_parity(*klasses, "# rbs_inline: enabled\n# encoding: ascii\n", config)
    end

    it "does not treat a non-enabled/disabled `rbs_inline` value as a magic comment" do
      source = "# rbs_inline: yes\n# encoding: ascii\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "flags an encoding comment after a `shareable_constant_value` comment" do
      expect_autocorrect_parity(*klasses,
                                "# shareable_constant_value: literal\n# encoding: ascii\n",
                                config)
    end
  end

  describe "bucket membership rules" do
    it "does not count a leading-space fsl line as an `other` magic comment" do
      source = "  # frozen_string_literal: true\n# encoding: ascii\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "pairs the encoding line with the LATEST preceding other magic comment" do
      expect_autocorrect_parity(
        *klasses,
        "# frozen_string_literal: true\n# shareable_constant_value: literal\n# encoding: ascii\n",
        config
      )
    end
  end

  describe "leading prefix boundaries" do
    it "keeps counting past a shebang before flagging the encoding line" do
      expect_autocorrect_parity(
        *klasses,
        "#!/usr/bin/env ruby\n# frozen_string_literal: true\n# encoding: ascii\n",
        config
      )
    end

    it "does not treat a magic-shaped hash literal after code as a leading comment" do
      source = "# frozen_string_literal: true\n\nx = { encoding: Encoding::SJIS }\nputs x\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end
end
