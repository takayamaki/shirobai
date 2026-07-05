# frozen_string_literal: true

# Print the badge name of every cop that shirobai replaces with a Rust
# implementation, as a single comma-separated line on stdout.
#
# Run it with the shirobai bundle so the wrapper cops are loaded:
#   BUNDLE_GEMFILE=benches/Gemfile.realconfig.shirobai \
#     bundle exec ruby benches/implemented_cops.rb
#
# The list is the source for real_cli_bench.sh's "removed" mode, which passes
# it to stock rubocop as --except to measure the theoretical upper bound of the
# replacement.
#
# We read the resolved registry, not the file names on disk: an earlier version
# camelized cop/<dept>/<name>.rb paths and picked up mixin modules that are not
# cops. Walking the registry and keeping the entries whose Ruby class lives
# under Shirobai:: is exact — requiring shirobai auto-enlists each wrapper class
# (RuboCop::Cop::Base.inherited), so the registry has one entry per real cop.

require "rubocop"
require "shirobai"

badges = RuboCop::Cop::Registry.global
  .select { |cop| cop.name.to_s.start_with?("Shirobai::") }
  .map(&:cop_name)
  .sort

puts badges.join(",")
