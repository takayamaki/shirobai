# frozen_string_literal: true

module Shirobai
  module Cop
    module Rails
      # Drop-in Rust reimplementation of `Rails/UnknownEnv`
      # (rubocop-rails 2.35.5).
      #
      # Rust detects the three `Rails.env` shapes (predicate / comparison /
      # `case`) and emits `[start, end, name]` per unknown environment. The
      # message — including the `DidYouMean` spell suggestion, which must stay
      # Ruby-side — is built here from this cop's own `Environments` config, so
      # the suggestion text is byte-for-byte stock. No autocorrect (stock has
      # none).
      #
      # The `supports_local` view (`local` is a known environment for the
      # predicate form only, and only on Rails >= 7.1) is packed into the
      # segment so the Rust filter matches; the comparison and `case` forms
      # never allow `local`, mirroring stock.
      class UnknownEnv < RuboCop::Cop::Base
        include Shirobai::Cop::BundleEligible

        MSG = "Unknown environment `%<name>s`."
        MSG_SIMILAR = "Unknown environment `%<name>s`. Did you mean `%<similar>s`?"

        def self.cop_name = "Rails/UnknownEnv"
        def self.badge = RuboCop::Cop::Badge.parse(cop_name)

        # Contributes `[nums, lists]` pieces to the rails segment:
        # `[[supports_local], [environments]]`.
        def self.bundle_args(config)
          cop = config.for_badge(badge)
          environments = cop["Environments"] || []
          supports_local = config.target_rails_version >= 7.1
          [[supports_local ? 1 : 0], [environments]]
        end

        def on_new_investigation
          buffer = processed_source.buffer
          source = bundle_eligible? ? processed_source.raw_source : buffer.source
          off = SourceOffsets.for(source)
          resolved_offenses.each do |start_offset, end_offset, name|
            range = Parser::Source::Range.new(buffer, off[start_offset], off[end_offset])
            add_offense(range, message: message(name))
          end
        end

        private

        def message(name)
          name = name.to_s.chomp("?")

          # DidYouMean::SpellChecker is not available in all versions of Ruby;
          # feature-check first, exactly like stock.
          similar_names = if defined?(DidYouMean::SpellChecker)
                            DidYouMean::SpellChecker.new(dictionary: environments).correct(name)
                          else
                            []
                          end

          if similar_names.empty?
            format(MSG, name: name)
          else
            format(MSG_SIMILAR, name: name, similar: similar_names.join(", "))
          end
        end

        def environments
          cop_config["Environments"] || []
        end

        def supports_local?
          target_rails_version >= 7.1
        end

        def resolved_offenses
          if bundle_eligible?
            Dispatch.offenses_for(processed_source, config, :rails_unknown_env)
          else
            Shirobai.check_rails_unknown_env(
              processed_source.buffer.source, environments, supports_local?
            )
          end
        end
      end
    end
  end
end
