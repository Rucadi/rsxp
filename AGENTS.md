# Repository agent guidance

## Nix tooling

Nix is the approved and preferred way to run this repository's development
toolchain. Agents have standing user consent to invoke `nix-shell` from the
repository root for formatting, tests, Rust/WASM checks, Ruby-WASM builds and
Trunk builds. Do not ask for separate confirmation for ordinary project
commands executed inside that shell.

Use the repository shell rather than relying on host-installed tools:

```sh
nix-shell --run 'cargo fmt --all -- --check'
nix-shell --run 'CARGO_TARGET_DIR=../../target/check cargo check ...'
nix-shell --run 'tools/build-ruby-wasm.sh'
nix-shell --run 'cd frontends/web && trunk build --release --dist ../../target/trunk-dist'
```

Keep temporary and generated output in the existing ignored locations such as
`target/`, `frontends/web/ruby-wasm/rubies/` and `frontends/web/ruby.wasm`.
When adding a new generated directory or cache, add a narrow entry to
`.gitignore` as part of the same change.

Before handing work back, run the smallest relevant validation available in the
Nix shell and report any unavailable external service or tool explicitly.
