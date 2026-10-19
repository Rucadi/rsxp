# iOS Simulator build and game testing

All commands assume the repository root (`/Users/rubencanodiaz/emulator/rsxp`).

## Build for the iOS Simulator

Xcode must be installed and its first-launch setup completed. Use the Nix shell
so the Rust toolchain and native build dependencies are available:

```sh
nix-shell
python3 mobile/build.py ios
```

The default build is Release. For a Debug build:

```sh
python3 mobile/build.py ios --debug
```

The app bundle is written to:

```text
mobile/build/ios/Build/Products/Release-iphonesimulator/RSXP.app
# or, with --debug:
mobile/build/ios/Build/Products/Debug-iphonesimulator/RSXP.app
```

## Build, install, and launch in one step

```sh
nix-shell
python3 mobile/build.py ios --run
```

`--run` creates an iPhone Simulator if none exists, boots it, installs the app,
and launches it (`org.rsxp.mobile`). Add `--debug` to build the Debug variant:

```sh
python3 mobile/build.py ios --debug --run
```

## Install and launch manually

List available simulators and boot one (replace the name with one from the list):

```sh
xcrun simctl list devices available
xcrun simctl boot "iPhone 16 Pro"
open -a Simulator
xcrun simctl bootstatus "iPhone 16 Pro" -b
```

Install the built app and launch it:

```sh
xcrun simctl install booted \
  mobile/build/ios/Build/Products/Release-iphonesimulator/RSXP.app
xcrun simctl launch booted org.rsxp.mobile
```

For a Debug build, change the path to
`mobile/build/ios/Build/Products/Debug-iphonesimulator/RSXP.app`.

You can also open `mobile/ios/RSXP.xcodeproj` in Xcode and run the `RSXP`
scheme on a simulator.

## Copy a game to the simulator

The app's game library is the `Games` folder inside its Documents container.
Get the container path, then copy the folder that directly contains `Game.ini`
and `Data` (not its parent directory):

```sh
GAME_DIR="/path/to/MyGame"          # folder containing Game.ini and Data/
CONTAINER="$(xcrun simctl get_app_container booted org.rsxp.mobile data)"

mkdir -p "$CONTAINER/Documents/Games"
cp -R "$GAME_DIR" "$CONTAINER/Documents/Games/"
```

The copied folder name becomes the game ID/title. Launch the app and tap the
game in the library, or launch it directly:

```sh
xcrun simctl terminate booted org.rsxp.mobile 2>/dev/null || true
xcrun simctl launch booted org.rsxp.mobile --game "$(basename "$GAME_DIR")"
```

Replace `booted` with a specific simulator UDID if several are running:

```sh
xcrun simctl list devices booted
xcrun simctl get_app_container <UDID> org.rsxp.mobile data
xcrun simctl launch <UDID> org.rsxp.mobile --game "MyGame"
```

The built-in demo can be launched without importing anything:

```sh
xcrun simctl launch --terminate-running-process booted org.rsxp.mobile --demo
```

Notes:

- Saves are stored per game under `Documents/Games/.saves` in the same
  container.
- Reinstalling the app or erasing the simulator removes its private games and
  saves, so recopy the game after a reset.
- To replace an already-imported game, remove the old folder first:
  `rm -rf "$CONTAINER/Documents/Games/MyGame"`.
- These launch arguments are handled by the iOS shell in both Debug and Release
  builds.
