# frozen_string_literal: true

require "spec_helper"

# Edge-case regression guard for `Layout/SpaceAroundEqualsInParameterDefault`.
#
# Quirks probed against stock that the vendor spec does not fully pin:
#
# - the offense range is `range_between(arg.end_pos, value.begin_pos)`; for a
#   signed-literal default (`-1` / `+1`) the value node begins AT the sign, so
#   the range stops right before it — matching parser-gem's third token;
# - `space_after?` is `/\G\s/` at the byte after the arg name / after the `=`,
#   so `y= 0` (space only after `=`) and `y =0` (space only before) both offend
#   under either style;
# - the cop is `on_optarg` only, so keyword defaults (`def f(a: 1)`) are never
#   touched;
# - block / lambda optargs (`->(y=0) {}`) are optargs too and are checked;
# - the autocorrect `/=\s*(\S+)/` remainder handling round-trips through both
#   styles.
RSpec.describe Shirobai::Cop::Layout::SpaceAroundEqualsInParameterDefault do
  include EdgeCaseParity

  def config_for(style)
    RuboCop::ConfigLoader.merge_with_default(
      RuboCop::Config.new(
        { "Layout/SpaceAroundEqualsInParameterDefault" => { "EnforcedStyle" => style } },
        "(test)"
      ),
      "(test)"
    )
  end

  klasses = [
    RuboCop::Cop::Layout::SpaceAroundEqualsInParameterDefault,
    Shirobai::Cop::Layout::SpaceAroundEqualsInParameterDefault
  ]

  describe "EnforcedStyle: space" do
    let(:config) { config_for("space") }

    it "corrects mixed missing/partial spacing" do
      expect_autocorrect_parity(*klasses, "def f(x, y=0, z= 1); end\n", config)
    end

    it "corrects defaults with unary +/- signed literals" do
      expect_autocorrect_parity(*klasses, "def f(x=-1, y= 0, z =+1); end\n", config)
    end

    it "corrects empty string / array / hash defaults" do
      expect_autocorrect_parity(*klasses, "def f(a=\"\", b=[], c={}); end\n", config)
    end

    it "checks block / lambda optargs too" do
      expect_autocorrect_parity(*klasses, "[].each { |y=0| y }\n", config)
      expect_autocorrect_parity(*klasses, "->(y=0) { y }\n", config)
    end

    it "accepts correctly spaced defaults" do
      source = "def f(x, y = 0, z = {}); end\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end

    it "does not touch keyword defaults" do
      source = "def f(a: 1, b:2); end\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end

  describe "EnforcedStyle: no_space" do
    let(:config) { config_for("no_space") }

    it "corrects spaced defaults down to no space" do
      expect_autocorrect_parity(*klasses, "def f(x, y = 0, z =1, w= 2); end\n", config)
    end

    it "corrects empty literal defaults with spaces" do
      expect_autocorrect_parity(*klasses, "def f(a = \"\", b = []); end\n", config)
    end

    it "accepts defaults without surrounding space" do
      source = "def f(x, y=0, z={}); end\n"
      expect_lint_parity(*klasses, source, config, expect_offenses: false)
      expect(lint_offenses(klasses.first, source, config)).to be_empty
    end
  end
end
