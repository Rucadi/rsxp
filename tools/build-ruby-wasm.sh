#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ruby_project="$repo_root/frontends/web/ruby-wasm"
output="${FANPLAY_RUBY_WASM_OUT:-$repo_root/target/ruby-wasm/ruby.wasm}"
target_dir="${FANPLAY_WASM_TARGET_DIR:-$repo_root/target/ruby-wasm/cargo}"
bundle_path="$ruby_project/vendor/bundle"

mkdir -p "$(dirname "$output")" "$target_dir"
export FANPLAY_RUST_DIR="$repo_root/cores/rgss/ruby"
export FANPLAY_WASM_TARGET_DIR="$target_dir"
export FANPLAY_WASM_OUT="$repo_root/target/ruby-wasm/extensions"
export BUNDLE_GEMFILE="$ruby_project/Gemfile"
export BUNDLE_PATH="$bundle_path"

cd "$ruby_project"
bundle install --jobs 4
(
  cd fanplay-rgss-wasm
  gem build fanplay-rgss-wasm.gemspec \
    --output "$repo_root/target/ruby-wasm/fanplay-rgss-wasm-0.1.0.gem"
)
# rbwasm hashes the gem directory, but the Rust sources live outside it.
# Re-run make/extconf so Cargo can track those sources instead of reusing a
# cached interpreter with stale native bindings.
bundle exec rbwasm build --ruby-version 4.0 --remake -o "$output"
cp "$output" "$repo_root/frontends/web/ruby.wasm"
printf 'built %s\n' "$output"
