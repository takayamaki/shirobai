# frozen_string_literal: true

require_relative "shirobai/version"

# Load the native extension. A fat/prebuilt platform gem nests the .so under a
# Ruby ABI directory (e.g. shirobai/3.4/shirobai.so); a source gem compiled
# locally puts it directly at shirobai/shirobai.so. Try the versioned path
# first, fall back to the flat one.
begin
  require_relative "shirobai/#{RUBY_VERSION[/\d+\.\d+/]}/shirobai"
rescue LoadError
  require_relative "shirobai/shirobai"
end

require_relative "shirobai/inject"
