# frozen_string_literal: true

require "set"

module Shirobai
  module Cop
    module RSpec
      # Drop-in Rust reimplementation of `RSpec/SubjectStub`
      # (rubocop-rspec 3.10.2).
      #
      # Architecture B (relocate-and-dispatch): the stock cop's cost lives in
      # the per-top-level-group subtree walks (`find_all_explicit` collects
      # every subject/let definition, then `find_subject_expectations`
      # recurses for message expectations) that it replays for EVERY top-level
      # group of every file. Rust runs that walk once on the shared traversal
      # and emits only the top-level groups that COULD hold an offense -- a
      # safe superset: a group qualifies when its subtree has a
      # message-expectation (`{allow|expect}(NAME).to <...receive...>` /
      # `is_expected.to <...receive...>`) whose target is `subject`,
      # `is_expected`, or a sym-named subject defined anywhere in the group.
      # Scope precision and `let` shadowing (which only REMOVE offenses) are
      # left to the replay here. Measured density: ~0.3% of Mastodon spec
      # files hold a candidate, so the expensive two-phase walk is skipped
      # almost everywhere.
      #
      # Candidate ranges are intersected with stock's OWN `top_level_groups`
      # (the `TopLevelGroup` mixin's selection, copied verbatim below) rather
      # than relocated via NodeLocator: the top-level test costs only a scan
      # of the root statements, and running it here keeps the top-level
      # semantics byte-for-byte stock even where the Rust spine
      # over-approximates (e.g. prism `BeginNode` = parser `kwbegin`, which
      # stock does NOT unwrap). A candidate that stock would not treat as a
      # top-level group simply never matches a verified group's range.
      #
      # The verified groups are processed by `process_candidate` -- stock's
      # `on_top_level_group` copied verbatim (the subject/let collection and
      # the `message_expectation?` node-pattern matching are deep parser-AST
      # structural work that cannot be reproduced bytewise). Detection only --
      # stock has no autocorrect.
      #
      # Stock's `TopLevelGroup` mixin is deliberately NOT included: its
      # `on_new_investigation` would drive `on_top_level_group` for every
      # top-level group unconditionally, which is exactly the cost this cop
      # removes.
      class SubjectStub < RuboCop::Cop::Base
        include RuboCop::RSpec::Language
        include Shirobai::Cop::BundleEligible

        MSG = "Do not stub methods of the object under test."

        def self.cop_name = "RSpec/SubjectStub"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Config-less (the segment's role lists cover everything).
        def self.bundle_args(_config)
          []
        end

        # --- stock node-pattern matchers, copied verbatim (rubocop-rspec 3.10.2) ---

        # @!method subject?(node)
        def_node_matcher :subject?, <<~PATTERN
          (block
            (send nil?
              { #Subjects.all (sym $_) | $#Subjects.all }
            ) args ...)
        PATTERN

        # @!method let?(node)
        def_node_matcher :let?, <<~PATTERN
          (block
            (send nil? :let (sym $_)
            ) args ...)
        PATTERN

        # @!method message_expectation?(node, method_name)
        def_node_matcher :message_expectation?, <<~PATTERN
          (send
            {
              (send nil? { :expect :allow } (send nil? %))
              (send nil? :is_expected)
            }
            #Runners.all
            #message_expectation_matcher?
          )
        PATTERN

        # @!method message_expectation_matcher?(node)
        def_node_search :message_expectation_matcher?, <<~PATTERN
          (send nil? {
            :receive :receive_messages :receive_message_chain :have_received
            } ...)
        PATTERN

        def on_new_investigation
          RuboCop::RSpec::Language.config = config["RSpec"]["Language"]
          ranges = resolved_candidates
          return if ranges.nil? || ranges.empty?

          source = bundle_eligible? ? processed_source.raw_source : processed_source.buffer.source
          off = SourceOffsets.for(source)
          keys = ranges.map { |(start, fin)| [off[start], off[fin]] }.to_set

          top_level_groups.each do |node|
            expr = node.source_range
            process_candidate(node) if keys.include?([expr.begin_pos, expr.end_pos])
          end
        end

        private

        # Bundle path only when raw_source and the parser buffer agree byte for
        # byte; a gated-off file (nil from Dispatch) falls back to the standalone
        # entry scanning `buffer.source`.
        def resolved_candidates
          if bundle_eligible?
            candidates = Dispatch.offenses_for(processed_source, config, :rspec_subject_stub)
            return candidates unless candidates.nil?
          end
          Shirobai.check_rspec_subject_stub(
            processed_source.buffer.source, *Shirobai::RSpec.segment(config)
          )
        end

        # --- stock TopLevelGroup selection, copied verbatim (rubocop-rspec 3.10.2) ---

        def top_level_groups
          @top_level_groups ||=
            top_level_nodes(root_node).select { |n| spec_group?(n) }
        end

        def top_level_nodes(node)
          return [] if node.nil?

          case node.type
          when :begin
            node.children
          when :module, :class
            top_level_nodes(node.body)
          else
            [node]
          end
        end

        def root_node
          processed_source.ast
        end

        # Stock `on_top_level_group`, renamed so the Commissioner never
        # dispatches it and driven from the verified candidates instead.
        def process_candidate(node)
          @explicit_subjects = find_all_explicit(node) { |n| subject?(n) }
          @subject_overrides = find_all_explicit(node) { |n| let?(n) }

          find_subject_expectations(node) do |stub|
            add_offense(stub)
          end
        end

        # --- stock private methods, copied verbatim (rubocop-rspec 3.10.2) ---

        def find_all_explicit(node)
          node.each_descendant(:block).with_object({}) do |child, h|
            name = yield(child)
            next unless name

            outer_example_group = child.each_ancestor(:block).find do |a|
              example_group?(a)
            end

            h[outer_example_group] ||= []
            h[outer_example_group] << name
          end
        end

        def find_subject_expectations(node, subject_names = [], &block)
          subject_names = [*subject_names, *@explicit_subjects[node]]
          subject_names -= @subject_overrides[node] if @subject_overrides[node]

          names = Set[*subject_names, :subject]
          expectation_detected = message_expectation?(node, names)
          return yield(node) if expectation_detected

          node.each_child_node(:send, :def, :block, :begin) do |child|
            find_subject_expectations(child, subject_names, &block)
          end
        end
      end
    end
  end
end
