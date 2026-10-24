# Core conformance and performance checks

This document describes the checks that exist in the tree. It does not claim
measured performance that has not been recorded.

## Contract and routing

```sh
cargo test -p rsxp-core -p rsxp-frontend
cargo test -p rsxp-core-gba
cargo test -p rsxp-cores --no-default-features --features gba
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
# Core/GBA only (no MRI or RGSS)
cargo check -p rsxp-desktop --no-default-features --features gba

# DS feature (unstable Dust dependencies)
RUSTC_BOOTSTRAP=1 cargo test -p rsxp-core-nds

# Full desktop/RGSS build
cargo test -p rsxp-desktop --bin rsxp
```

The desktop `gba` feature has no dependency edge to `rsxp-rgss`. Verify it:

```sh
cargo tree -p rsxp-desktop --no-default-features --features gba -e normal \
  | grep -E 'rsxp-rgss|rsxp-rgss-ruby|magnus' && exit 1 || true
```

CI runs this matrix on Ubuntu. The DS feature requires a nightly compiler (or
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
cargo run --release -p rsxp-desktop --example session-bench -- target/release/rsxp

# Headless core throughput; report frame time and audio queue status rather
# than assuming the backend allocation profile.
cargo run --release -p rsxp-core-runner -- --content game.gba --frames 600
```

The cartridge bridge keeps steady-state video and audio memory bounded and uses
a preallocated SPSC PCM ring. Backend allocation is measured separately; the
frontend does not assume the emulator backend is allocation-free.
