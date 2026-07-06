# frozen_string_literal: true

require "spec_helper"

# Plumbing specs for the rails packed-config segment and origin registration
# (no cop behavior here — cop specs live next to each wrapper). The rails
# origin has no per-file gate, so the segment is a constant wake-up flag.
RSpec.describe Shirobai::Rails do
  describe ".segment" do
    let(:config) { RuboCop::ConfigLoader.default_configuration }

    it "packs the wake-up flag plus the send/block-table cops' config" do
      nums, lists = described_class.segment(config)
      # nums = [enabled, unknown_env_supports_local]
      expect(nums.length).to eq(2)
      expect(nums.first).to eq(1)
      # lists = [environments, allowed_methods, allowed_receivers, whitelist]
      expect(lists.length).to eq(4)
      expect(lists[0]).to include("development", "test", "production")
    end
  end

  describe "the registered packer" do
    it "is registered for the :rails origin with no gate" do
      expect(Shirobai::Dispatch.plugin_packers[:rails]).not_to be_nil
      # No per-file gate: rails cops run on every Ruby file.
      expect(Shirobai::Dispatch.plugin_gates[:rails]).to be_nil
    end

    it "adds :rails as the fourth origin" do
      expect(Shirobai::Dispatch::ORIGINS).to eq(%i[core performance rspec rails])
    end
  end

  describe "check_all through the token" do
    let(:config) { RuboCop::ConfigLoader.default_configuration }

    it "computes the four Application* slots on an awake token" do
      token = Shirobai::Dispatch.bundle_token(config)
      result = Shirobai.check_all(
        "class Foo < ActiveRecord::Base\nend\n" \
        "class C < ActionController::Base\nend\n" \
        "class M < ActionMailer::Base\nend\n" \
        "J = Class.new(ActiveJob::Base)\n",
        token
      )
      expect(result.length).to eq(Shirobai::Dispatch::ORIGINS.length)
      # rails origin = 3, slots 0..3.
      expect(result[3][0]).to eq([[12, 30]]) # ApplicationRecord: `ActiveRecord::Base`
      expect(result[3][1].length).to eq(1)   # ApplicationController
      expect(result[3][2].length).to eq(1)   # ApplicationMailer
      expect(result[3][3].length).to eq(1)   # ApplicationJob
    end

    it "keeps the rails slots empty on a dormant (forced-inactive) token" do
      token = Shirobai::Dispatch.bundle_token(config, %i[rails].freeze)
      result = Shirobai.check_all("class Foo < ActiveRecord::Base\nend\n", token)
      expect(result.length).to eq(Shirobai::Dispatch::ORIGINS.length)
      expect(result[3]).to eq([[], [], [], [], [], []])
    end
  end

  describe "the dormant segment token" do
    it "matches the documented layout" do
      expect(Shirobai::Dispatch::DORMANT_SEGMENTS.fetch(:rails))
        .to eq([[0, 0], [[], [], [], []]])
    end
  end

  describe "the standalone fallback entry points" do
    it "returns the same ranges as the bundle for each cop" do
      source = "class Foo < ActiveRecord::Base\nend\nBaz = Class.new(ActionController::Base)\n"
      expect(Shirobai.check_rails_application_record(source)).to eq([[12, 30]])
      expect(Shirobai.check_rails_application_controller(source).length).to eq(1)
      expect(Shirobai.check_rails_application_mailer(source)).to eq([])
      expect(Shirobai.check_rails_application_job(source)).to eq([])
    end
  end
end
