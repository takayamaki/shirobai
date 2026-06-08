# frozen_string_literal: true

require "rbconfig"

DLEXT = RbConfig::CONFIG["DLEXT"]

task :compile do
  sh "cargo build --release -p shirobai-ext"
  mkdir_p "lib/shirobai"
  cp "target/release/libshirobai.#{DLEXT}", "lib/shirobai/shirobai.#{DLEXT}"
end

task default: :compile
