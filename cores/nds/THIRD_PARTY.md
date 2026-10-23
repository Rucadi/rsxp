# Third-party sources

- Dust: https://github.com/kelpsyberry/dust, Cargo dependencies follow branch
  `main`; Cargo.lock currently resolves to
  `76d79f25e4ea3ce355c4ac868f5ac9a233e64b89`. GPL-3.0 (see `DUST_LICENSE`).
  Core and 2D renderer are Cargo git dependencies. `core/game_db.json` is
  copied unchanged from the locked revision; upstream credits melonDS for this
  database. `vendor/dust-soft-3d` is the software 3D renderer from the same
  revision, with its core dependency pointing at the `main` branch,
  `Mask::to_int()` changed to `to_simd()`, and the new `Select` trait imported.
- emu-utils: https://github.com/kelpsyberry/emu-utils, revision
  `fff4bcd769b2fd1b452d79aad7d7741589e64c0a`.
  `vendor/emu-utils` retains the upstream source and macro crate. Compatibility
  changes remove obsolete SIMD lane-count bounds, obsolete rustc scalar-layout
  attributes, and the stabilized `new_zeroed_alloc` feature gate. The bounded
  integer constructors and range checks are retained. Upstream does not carry
  a standalone license file in this revision.
- arm-decoder is a transitive git build dependency pinned by `Cargo.lock`.

These local Cargo patches adapt the unstable APIs to Rust 1.97. They avoid
editing the user's global Cargo cache and apply only within this workspace.
Do not remove the patches without testing the pinned upstream dependencies
against the selected nightly toolchain.
