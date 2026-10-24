# Simplification baseline

The codebase simplification work started from revision
`9c4b8ffecb6c2fa4954f3c6281ae2b1da11798b0` on 2026-10-24. The working tree
was clean before the implementation began.

This baseline records reproducible correctness gates, not performance claims.
Performance comparisons still require the fixed games, warmup, renderer,
platform and input streams described in
[codebase-simplification-plan.md](codebase-simplification-plan.md).

## Ownership at the baseline

- `rsxp-core` owns contracts and validation.
- `rsxp-frontend` owns routing, storage, native worker lifecycle and portable
  frame conversion.
- `rsxp-cores` owns feature-selected static registration.
- Core crates own machine- and engine-specific execution.
- `rsxp-ui` remains the shared RGSS workspace and presentation application.
- Desktop and browser hosts own scheduling, device APIs and transport.

## Required correctness gates

Run the validation checklist in the simplification plan. At minimum, changes
to portable frontend services must pass `cargo test -p rsxp-core
-p rsxp-frontend`, and feature-boundary changes must pass the GBA-only desktop
check plus a dependency-tree inspection.
