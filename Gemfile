# frozen_string_literal: true

source "https://rubygems.org"

gemspec

gem "rake", "~> 13.0"
gem "rake-compiler"
gem "rspec", "~> 3.0"

# Optional project-index backend (AllCops/UseProjectIndex). Dev-only: the
# index-aware vendor spec contexts skip without it, and shirobai does not
# depend on it at runtime — index-aware wrapper paths only run when RuboCop
# hands the cop a non-nil project_index.
gem "rubydex"
