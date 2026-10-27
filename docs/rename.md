# Fanplay naming and existing data

The product, executables, Cargo packages, crate directories, frontend bridges,
build scripts and documentation use Fanplay names. The RGSS package is now
`fanplay-rgss` (Rust import `fanplay_rgss`); the desktop executable is `fanplay`.
Environment variables use `FANPLAY_*`, Ruby/JS bridges use `FANPLAY`, and new
plugin metadata uses `// fanplay:...`.

## Data compatibility

- `FANPLAY_HOME` selects native storage. `RSXP_HOME` remains a fallback when the
  new variable is absent.
- At a portable/home directory, an existing `RSXP` folder is reused if no
  `FANPLAY` folder exists. Installed games inside either folder inherit that
  library. No directory is renamed or deleted automatically.
- Legacy native library/controller locations and title-based saves remain
  migration inputs. The standalone core runner also reuses an existing legacy
  save root when its new root is absent.
- Core IDs, state revisions, content hashes, `RSXPSAV` envelope magic and
  `.rsxpstate` suffix remain stable. A product rename alone must not invalidate
  emulator save states.

Other old environment variables and custom plugin/Ruby/JS integration names
are not universal aliases. Update external launch scripts to `FANPLAY_*`,
plugin headers to `fanplay:`, and custom bridge calls to `FANPLAY`. Bundled
plugins and tests are updated together with the host.

The checkout directory and Git remote are not renamed. The existing external
Cachix cache defaults to `rsxp` until the repository's `CACHIX_CACHE` variable
selects a provisioned replacement. These deliberate legacy names are not
missed product-brand substitutions. Raw historical benchmark captures also
retain the names, paths and commands recorded at measurement time.
