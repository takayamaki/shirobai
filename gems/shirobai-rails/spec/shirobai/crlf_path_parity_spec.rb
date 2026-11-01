# frozen_string_literal: true

require "spec_helper"

# rubocop-rails half of the core suite's spec/shirobai/crlf_path_parity_spec.rb:
# a CRLF file must take the same path as its LF original.
RSpec.describe "CRLF source path parity (Rails cops)" do
  include StandalonePathParity

  let(:cops) { wrapper_cops(%i[Rails]) }
  let(:config) { config_enabling(cops) }

  let(:source) do
    <<~RUBY
      # frozen_string_literal: true

      class User < ActiveRecord::Base
      end

      class UsersController < ActionController::Base
        def show
          get :show, { id: 1 }
          User.find_by_name_and_email(name, email)
          user.errors[:name] << "msg"
          users.map { |u| u[:id] }
          users.map { |u| [u.id, u] }.to_h
          users.to_h { |u| [u, u.name] }
          Rails.env.proudction?
        end
      end

      class UserMailer < ActionMailer::Base
      end

      class UserJob < ActiveJob::Base
      end
    RUBY
  end

  it "calls the same standalone entry points on a CRLF copy" do
    expect(path_diverging_cops(cops, source, config)).to eq({})
  end
end
