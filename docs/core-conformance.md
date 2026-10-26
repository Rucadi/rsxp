# Core conformance and performance checks

This document describes the checks that exist in the tree. It does not claim
measured performance that has not been recorded.

## Contract and routing

```sh
cargo test -p fanplay-core -p fanplay-frontend
cargo test -p fanplay-core-gbc -p fanplay-core-gba
cargo test -p fanplay-cores --no-default-features --features gbc,gba
```

The core/frontend suite covers routing, command request ids and payload bounds,
panic-safe completion, terminal events, content identity, state envelopes,
storage migration and in-memory storage. The GBA suite includes a headless
boot/reset test, state production, and a save/advance/restore/replay check that
rejects a corrupted envelope without changing the running machine.

GBA frame replay uses the same fixed ROM and input snapshot before save and
after restore; frame hashes must match. The DS suite performs the same
transactional restore check with its candidate-machine path.

## Feature matrix

```sh
# Core cartridges only (no MRI or RGSS)
cargo check -p fanplay-desktop --no-default-features --features gbc,gba

# DS feature (unstable Dust dependencies)
RUSTC_BOOTSTRAP=1 cargo test -p fanplay-core-nds

# Full desktop/RGSS build
cargo test -p fanplay-desktop --bin fanplay
```

The desktop cartridge features have no dependency edge to `fanplay-rgss`. Verify it:

```sh
cargo tree -p fanplay-desktop --no-default-features --features gbc,gba -e normal \
  | grep -E 'fanplay-rgss|fanplay-rgss-ruby|magnus' && exit 1 || true
```

These are local validation commands; see `.github/workflows/build.yml` for
the checks actually configured in CI. The DS feature requires a nightly compiler (or
`RUSTC_BOOTSTRAP=1` for local checks) because of the Dust dependency.

## Save and storage failures

- Same-title content is stored under distinct content hashes.
- Resume and state-load paths reject wrong core ids, revisions, content hashes,
  truncation, oversized payloads and checksum failures before restore.
- Storage runs on a bounded worker with atomic replacement. A full queue returns
  a typed error, and failed writes are counted and reported at session end.
- Legacy title directories are copied forward; the originals are preserved.

## Performance measurements

Measurements must be recorded against a fixed benchmark host before a
regression or improvement is claimed. Useful commands:

```sh
# Isolated RGSS transport baseline.
cargo run --release -p fanplay-desktop --example session-bench -- target/release/fanplay

# Headless core throughput; report frame time and audio queue status rather
# than assuming the backend allocation profile.
cargo run --release -p fanplay-core-runner -- --content game.gba --frames 600
```

The cartridge bridge keeps steady-state video and audio memory bounded and uses
a preallocated SPSC PCM ring. Backend allocation is measured separately; the
frontend does not assume the emulator backend is allocation-free.
