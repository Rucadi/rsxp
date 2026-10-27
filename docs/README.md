# Fanplay documentation

The current guides below were reconciled with this checkout during the Fanplay
rename (2026-10-26). Source paths are provided so behavior can be checked when
implementation changes. Implemented features, host limitations, and future
work are described separately.

| Topic | Guide |
| --- | --- |
| Start here | [Project README](../README.md) |
| Ownership and execution paths | [Architecture](architecture.md) |
| Core contract, capabilities and saves | [Multi-core architecture](multi-core-architecture.md) |
| Toolchains and checks | [Development](development.md), [core conformance](core-conformance.md) |
| Native workspace | [Dashboard](dashboard.md) |
| Shared UI and host boundaries | [Shared frontend](shared-frontend.md) |
| Browser/WASM frontend | [Web build plan](web-build-plan.md) |
| RPG Maker compatibility | [Runtime](rust-port.md) |
| GBA / DS behavior | [GBA](../cores/gba/README.md), [DS](../cores/nds/README.md) |
| Library, storage and downloads | [Provisioning](software-provision.md) |
| Local API and MCP | [Control API](control-api.md), [MCP](../crates/fanplay-mcp/README.md) |
| Rhai and game-specific tools | [Plugins](../extensions/README.md) |
| Names and existing data | [Rename compatibility](rename.md) |
| Visual direction | [Design system](../DESIGN.md) |

## Historical evidence

The performance notes record particular workloads, machines and past revisions.
Raw JSON captures under `docs/benchmarks` remain unchanged as historical
evidence, including their original names and paths. They are not a current
support matrix or guaranteed frame rate:
[renderer](fanplay-performance.md), [tilemaps](native-tilemap-performance.md),
[retained state](retained-render-state.md), [session transport](session-performance.md),
[Pokémon Z](pokemonz-performance.md).

[The simplification baseline](simplification-baseline.md) is historical.
[Remaining boundary work](codebase-simplification-plan.md) and
[core improvement notes](core-design-improvement-plan.md) distinguish future
work from the implemented architecture.
