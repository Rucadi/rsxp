# Game workspace

Launching `rsxp` opens a persistent egui dashboard. `rsxp run --game DIRECTORY`
opens that dashboard with a game selected; `demo` and headless CLI commands keep
their existing behavior. The library accepts a directory or Game.ini, supports
drag-and-drop, and remembers twelve recently opened games in
`$XDG_CONFIG_HOME/rsxp/library.json` (falling back to `$HOME/.config`).

There is one active game. Opening another replaces it. Stop and restart leave
the dashboard open. The selected game's tools, textures, requests, and runtime
settings are cleared when switching games. The sidebar presents Runtime,
Pokémon Essentials, pokeAPI, and Online play separately; performance and runtime
output are optional workspace panels. Pokémon Essentials contains its bundled
patches, Load world map, Current-map encounters, Pokédex, Movedex, Itemdex, and a
Debug mode toggle that reads and writes the game's own `$DEBUG` flag. The world map and encounters
open their dedicated utility windows. Other map controls, party/status/NPC/battle
requests live in pokeAPI. Cable Club connections have a dedicated Online play tab.
pokeAPI opens an API and testing window with controls and the complete OpenAPI
contract on the left, and the response on the right. Text is selectable for
Ctrl+C copying, and the response and OpenAPI contract also have copy buttons.

The Pokédex has a fuzzy-searchable species/form list (name, species ID or numeric
Pokédex ID), independent scrolling, and a pinned identity header. Overview shows
base stats, abilities and species facts; Locations shows wild encounter maps,
level ranges, versions and slot shares; Moves filters complete level-up, TM/tutor
and egg learnsets; Evolution shows game-specific conditions and links to relatives.
Its data comes from the running game's registries, including custom species and
changes. Older Essentials releases without the `GameData::*` registries (such as
Pokémon Z) are read from their compiled `PBSpecies`/`PBMoves`/`PBItems` modules
and flat data files through the same explorer. Known acquisition sources include
wild encounters, party, evolution and
breeding. The optional source filter does not claim to detect scripted gifts,
trades, quests or every conditional form. Missing evidence is labeled as unknown.
Refresh rebuilds the session cache, and explorer requests preserve API responses.

The Pokédex also has a Cheats tab that starts a wild encounter with the selected
species and optional level, shiny, form, gender, nature, ability, held item, max
IVs and friendship. It only runs from the overworld with no battle, menu or
dialogue open, and the battle starts on the next game frame. The same operation is
available as `POST /v1/pokemon-essentials/wild-encounter` and the
`pokemon_essentials_wild_encounter` MCP tool.

Movedex and Itemdex use the same fuzzy name/ID search as Pokédex, with bounded
panes and virtualized result lists. Movedex combines type, category, Has TM, power,
accuracy, PP, priority and flag filters and numeric sorting. Itemdex filters bag
pockets, item kinds, consumable status, price, usage and flags. Both show the
running game's descriptions and values and support Ctrl+C and full-entry copying.
Itemdex can add the selected item and quantity to the current bag. Movedex offers
an add-TM action only for game-defined TMs teaching that move, with a selector
when several exist. These actions run on the game thread through bag methods,
check capacity, disable duplicate clicks while pending, and report the actual
quantity added and the resulting total. Their results remain in the dex window.

On desktop, Runtime lists user Ruby files from the selected game's `patches/`
folder. Files are discovered when starting a session and start disabled. Use
letters, digits, underscores, or hyphens in filenames; each `.rb` file must return
an `apply`/`revert` hash of procs, for example:

```ruby
{ apply: proc { puts "Enabled" }, revert: proc { puts "Disabled" } }
```

Enable them in Runtime or pre-enable with `RSXP_PATCH=user/filename`. The bundled
Essentials patches keep their existing IDs and appear in Pokémon Essentials.

Gameplay starts with only the game visible. The library remains available when
no game is running. F1 toggles the entire workspace, including the session header,
toolbar, panels and inspector windows; hidden controls leave no restore button
or floating-window chrome over the game. Entering desktop fullscreen also hides
the workspace; F1 can reveal it again while fullscreen remains active.

| Shortcut | Action |
| --- | --- |
| F1 | Show/hide the entire workspace UI |
| F2 | Show UI and toggle Tools |
| F3 | Show UI and toggle Runtime output |
| F4 | Toggle desktop fullscreen |
| F10 | Show UI and toggle Performance |
| F11 | Toggle uncapped execution |

These workspace shortcuts are reserved rather than forwarded to the game.
Browser controls share F1/F2/F3/F10/F11; F4 is a desktop fullscreen control.

Pause suspends game updates and mutes audio; audio playback positions continue
advancing. Inspector requests wait until resumed.

When a game exits, its final stdout/stderr messages are retained in Runtime
output. A stopped session shows that panel automatically, including when output
was hidden during play. The same streams are forwarded to the terminal that
launched rsxp, and failed exits include their process status. The dashboard stays
open so the diagnostics can be read or copied before starting another game.
The GUI retains the most recent 300 lines (up to 4096 characters per line);
terminal forwarding preserves the original bytes.

## Ownership and portability

- The main thread owns winit, the surface, presentation, and egui.
- Each native launch starts the same executable as an isolated runner. MRI is
  initialized once in that process and is never reinitialized. Working directory,
  environment, Ruby globals, and media belong to the runner.
- A dedicated I/O thread exchanges controls, inspector messages, metrics, and
  bounded render submissions / software framebuffers over a nonce-authenticated loopback connection.
  Only the newest received frame is retained for presentation. Game execution
  cannot hold the UI thread; an unresponsive runner can be stopped independently.
  A bounded reader delivers controls asynchronously after the initial handshake;
  game updates never wait for a per-frame UI/control reply.
- `ui::backend::Backend` is the injected panel service contract. The shared UI
  has no direct Ruby, filesystem, runtime-setting, or transport operations.
  `workspace_services::LocalServices` adapts the existing runtime services.
  Game asset decoding uses a bounded background queue.
- Each panel lives in its own file under `crates/rsxp-rgss/src/ui/`. Game-specific inspectors are
  kept separate from the library, session controls, saves, and game view.

The browser keeps its existing cooperative Ruby/WASM execution and loader. It
uses the same panel modules and local service adapter without the desktop
launcher or subprocess transport. Web save export/import remains available.

The native GPU path transports draw commands, tile geometry, and revision-based
asset uploads. The dashboard composes directly on its presentation GPU: ordinary
game frames perform no GPU readback and do not upload a finished framebuffer.
Queued views merge without losing ordered asset updates. Explicit screenshots,
freeze, and snap operations materialize the runner's current scene on demand.
Software fallback still transports pixels, using a bulk write and uploading only
when the received bitmap revision changes. See [transport performance](session-performance.md)
for the benchmark and regression checks.

## Save files

The Saves view copies an explicitly selected save directory to an empty backup
directory. For games using `System.data_directory`, an existing conventional
save directory is suggested. Custom game save locations can be entered manually.
The whole selected directory is copied; do not select the game installation.
Cartridge saves are content-scoped (`<data>/rsxp/saves/<core>/<hash>`) and are
not managed by this RGSS directory picker.

Restore stages incoming files, preserves originals under `before-restore-*`,
and atomically replaces individual matching files. Extra files remain. Symlinks,
overlapping directories, and incompatible destination types are rejected.
The game must be stopped, and copying runs off the UI thread. This manages game
save files, not VM snapshots or arbitrary save-format editing. A restore can be
partly committed if a filesystem error occurs; the original files remain in its
safety backup.

## Future Android work (no implementation)

The subprocess and native path adapters are desktop details. An Android frontend
must provide a runtime backend compatible with its Ruby embedding constraints,
document-provider storage handles, lifecycle/surface recreation, touch input,
and compact navigation. Shared panels should depend on capabilities and opaque
storage identifiers when that adapter is introduced. Arbitrary desktop paths,
extra native windows, and subprocess availability must not become requirements
of the shared UI. Independent multi-VM support is outside the single-game design.

Android support, document pickers, and lifecycle integration are not implemented
by this change. Native library and directory-backup controls currently use path
text fields behind the shared session intent interface.
