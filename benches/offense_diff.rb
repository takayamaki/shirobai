# frozen_string_literal: true

# Compare two rubocop JSON outputs (stock vs shirobai).
# Prints total counts, then fails when the offense sets differ.
# Unlike parity_diff.sh this is meant for runs with the corpus's own config,
# where offenses can exist — the check is that both sides report the same set.
#
# Usage: ruby benches/offense_diff.rb <stock.json> <shirobai.json>

require "json"

def load_offenses(path)
  data = JSON.parse(File.read(path))
  counts = Hash.new(0)
  keys = {}
  data["files"].each do |file|
    file["offenses"].each do |o|
      cop = o["cop_name"]
      key = "#{file["path"]}|#{o["location"]["line"]}:#{o["location"]["column"]}|#{o["message"]}"
      counts[cop] += 1
      (keys[cop] ||= {})[key] = true
    end
  end
  [data["files"].size, data["summary"]["offense_count"], counts, keys]
end

st_files, st_total, st_counts, st_keys = load_offenses(ARGV.fetch(0))
sh_files, sh_total, sh_counts, sh_keys = load_offenses(ARGV.fetch(1))

puts format("stock    files=%-6d offenses=%d", st_files, st_total)
puts format("shirobai files=%-6d offenses=%d", sh_files, sh_total)

cops = (st_counts.keys + sh_counts.keys).uniq.sort
diffs = cops.reject { |c| st_counts[c] == sh_counts[c] && st_keys[c] == sh_keys[c] }

if diffs.empty? && st_total == sh_total
  puts "offense diff: NONE (both sides report the same offenses)"
else
  puts "offense diff (#{diffs.size} cops):"
  diffs.each do |c|
    only_sh = ((sh_keys[c] || {}).keys - (st_keys[c] || {}).keys).size
    only_st = ((st_keys[c] || {}).keys - (sh_keys[c] || {}).keys).size
    printf("  %-50s shirobai=%-6d stock=%-6d  (sh-only=%d, st-only=%d)\n",
           c, sh_counts[c], st_counts[c], only_sh, only_st)
  end
  exit 1
end
