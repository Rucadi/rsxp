# RSXP library and software provisioning

The desktop launcher opens a searchable game library with platform filters, local
file/folder launch, repository management, and a storage page. Connect multiple
local JSON files or HTTPS catalog URLs under **Repositories**. Catalogs are
stored permanently in `config/config.json`, can be selected or deselected for
the library, and retain their last verified cache when a refresh fails. Select
**Download & play** to install and launch through the existing RPG Maker, GBA,
or NDS runtime. Downloads and hashing run in a background worker. Errors remain
visible and can be retried; cached catalogs work offline. Remote downloads
require `curl` on PATH. TLS verification is enabled and redirects are restricted
to HTTPS.

## The RSXP folder

Storage selection, in priority order:

1. `RSXP_HOME`, if set (use an absolute path).
2. For a game: its enclosing `RSXP` installation, or an existing `RSXP` folder in
   its directory (beside a cartridge file).
3. An existing `RSXP` folder beside the launcher executable.
4. `~/RSXP` (`%USERPROFILE%/RSXP` on Windows when HOME is unset).

Create the folder before launching to opt into portable mode. The launcher shows
its active path in **Storage & saves**. A game-specific portable folder affects
that game's runtime; the launcher catalog remains in the launcher's own root.

```
RSXP/
  config/config.json                # common/per-game settings, catalogs, API
  config/library.json
  config/games/<hash>.json          # legacy overrides, migrated on save
  repositories/sources.json         # legacy source list, migrated once
  repositories/cache/<catalog>.json
  games/<repository-id>/<game-id>/
  roms/<sha256>.gba                 # or .nds
  saves/gba/<content-id>/save/
  saves/gba/<content-id>/states/
  saves/nds/<content-id>/save/
  saves/nds/<content-id>/states/
  saves/rgss/<game-title>/
  backups/<game-key>/<snapshot>/
  cache/
```

`config/config.json` is the installation-wide document. It is validated on load;
a corrupt file is moved aside as `config.json.corrupt-<pid>` and a fresh default
is used so the launcher still starts. The loopback control API defaults to
`127.0.0.1:47821`; change it in `config/config.json`, the API panel, or with
`RSXP_API_PORT`. See [unified control API](control-api.md).

Old library preferences and saves are read/copied from their previous locations
without deleting the originals. RPG Maker games that write relative save files
continue writing inside their game folder: provisioned games keep those inside
`RSXP/games`. Externally opened projects retain their existing project-local save
behavior. RPG Maker titles using `System.data_directory` use `RSXP/saves/rgss`.

## Repository schema (version 1)

See [the example catalog](repositories/example.json). It is a template with
placeholder hashes and paths, not a downloadable game catalog. Generate real
hashes using `sha256sum FILE` (or an equivalent SHA-256 tool).

Each repository has `schema_version`, unique `id`, display `name`, and `games`.
Each game has:

- `id`: unique inside the repository, ASCII letters, digits, `_` or `-`.
- `title`, `version`, optional `description`.
- `platform`: `rpg-maker`, `gba`, or `nds`.
- `download`: `url`, `sha256` (64 hexadecimal characters), and `format` (`rom`,
  `zip`, or `ips`). URLs may be HTTPS, or relative to the catalog. Local catalogs
  may also reference absolute local paths. Remote catalogs cannot read local files.
- `entrypoint`: safe relative path to the `.gba`, `.nds`, or `Game.ini` file.
  RPG Maker requires ZIP packaging; cartridge games support raw ROMs or ZIPs.
- For IPS: `base` with descriptive `title` and exact `sha256`, plus mandatory
  `output_sha256` for the resulting cartridge ROM. IPS is only for GBA/NDS.

Use **Storage & saves → Import base ROM** or put your own ROM under `RSXP/roms`.
RSXP finds the base by content hash, not filename. A missing or mismatched base
blocks installation and launch of an IPS entry. RSXP never downloads base ROMs.
It verifies the downloaded IPS, applies literal and RLE records to a copy,
supports the optional final output-size field, verifies the result, and installs
it atomically. It rechecks installed cartridge hashes before launching.

Downloads are limited to 512 MiB, catalogs to 8 MiB, and ZIP expansion to 2 GiB /
100,000 entries. Traversal paths and symlinks in ZIPs are rejected. Installation
uses a staging directory; failed verification never produces an installed game.
An existing different version is not overwritten because games may store saves
there. Back up and move that installation before installing another version.
Interrupted installations may leave `.GAME-staging` beside the installation;
remove that staging directory before retrying. There is currently one background
operation at a time, with status feedback rather than byte-level progress.

## Command line

```
rsxp software-provision list ./games.json https://example.org/other.json
rsxp software-provision import-base ./my-base.gba
rsxp software-provision install ./games.json game-id
```

`install` prints the entrypoint path. Run it with `rsxp run --game PATH`, or use
the desktop launcher to download and launch in one action. Repository hashes
ensure consistency with a catalog; choose repository publishers you trust.
