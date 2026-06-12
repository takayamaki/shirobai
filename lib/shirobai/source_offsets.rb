# frozen_string_literal: true

module Shirobai
  # Converts the BYTE offsets Rust reports (prism) into the CHARACTER offsets
  # `Parser::Source::Range` indexes the buffer with. On an ASCII-only source
  # the two coincide and `#[]` returns its argument unchanged (`ascii_only?`
  # is a cached coderange lookup, so the fast path costs one predicate per
  # offset); on a non-ASCII source each distinct offset is converted once via
  # `byteslice(0, offset).length` (O(offset), memoized — offenses are rare and
  # repeat offsets, so the naive prefix count beats maintaining a table).
  #
  # `.for` memoizes the instance by source identity (`equal?`, the same
  # single-slot scheme as `Dispatch`): cops run file by file, so every cop on
  # the current file shares one converter (and its memo), while the
  # autocorrect loop's freshly built source naturally gets a new one.
  #
  # NOTE for wrapper authors: convert ONLY what feeds `Parser::Source::Range`
  # (or other char-indexed Ruby APIs). Values handed BACK to Rust (e.g.
  # `IndentationWidth`'s accumulated prior correction ranges) must stay bytes.
  class SourceOffsets
    class << self
      def for(source)
        return @cached if defined?(@cached_source) && @cached_source.equal?(source)

        @cached_source = source
        @cached = new(source)
      end
    end

    def initialize(source)
      @source = source
      @ascii = source.ascii_only?
      @chars = nil
    end

    # The character offset for prism's `byte_offset` into this source.
    def [](byte_offset)
      return byte_offset if @ascii

      (@chars ||= {})[byte_offset] ||= @source.byteslice(0, byte_offset).length
    end
  end
end
