# Android build, emulator and game testing

All commands assume the repository root. The Android port targets arm64
(`arm64-v8a`). On an Intel Mac use a physical arm64 device or adapt the build for
`x86_64`; the bundled Ruby runtime is arm64 only.

## Development shell

`shell-droid.nix` supplies Android Studio, SDK 35, NDK r28, CMake 3.22.1,
Gradle 8, JDK 17, the arm64 emulator image, Rust and patchelf. Unfree packages
and the Android SDK licences are accepted by the shell expression:

```sh
nix-shell shell-droid.nix
```

## Build the debug APK

```sh
nix-shell shell-droid.nix
python3 mobile/build.py android
```

This downloads the pinned, SHA-256 verified Termux-patched Ruby 3.3 runtime on
the first build, compiles the Rust core for `aarch64-linux-android`, stages the
UI, Ruby runtime and native libraries, and runs Gradle.

Output:

```text
mobile/android/app/build/outputs/apk/debug/app-debug.apk
```

You can also open `mobile/android` in Android Studio and use **Build APK** after
the staging step above has created `mobile/build/android`.

If you have your own arm64 Ruby distribution, point `RSXP_RUBY_PREFIX` at a
prefix containing `include/ruby-*`, `lib/libruby.so*` and `lib/ruby/3.3.0`.

## Create and start an emulator

The shell contains the `system-images;android-35;google_apis;arm64-v8a` image.
Create an AVD once:

```sh
avdmanager create avd \
  -n rsxp \
  -k "system-images;android-35;google_apis;arm64-v8a" \
  -d pixel_7
```

Then start it and wait for it to boot:

```sh
emulator -avd rsxp -no-snapshot &
adb wait-for-device
adb shell 'while [ "$(getprop sys.boot_completed)" != "1" ]; do sleep 1; done'
```

Alternatively start the emulator from Android Studio's Device Manager.

## Install and launch

```sh
adb install -r mobile/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n org.rsxp.mobile/.MainActivity
```

Launch the built-in demo directly:

```sh
adb shell am start -n org.rsxp.mobile/.MainActivity --ez demo true
```

Launch a game that is already in the app library by folder name:

```sh
adb shell am start -n org.rsxp.mobile/.MainActivity --es game "MyGame"
```

## Copy a game for testing

### Through the app (normal path)

1. Push the folder that directly contains `Game.ini` and `Data` to the
   emulator's shared storage:

   ```sh
   adb push /path/to/MyGame /sdcard/Download/MyGame
   ```

2. Open RSXP, tap **Import game folder**, and choose `Download/MyGame` in the
   system picker.

The app copies the folder into its private library (`files/Games/MyGame`) and
leaves the source untouched. Pick the game folder itself, not its parent.

### Directly into the debug app's private library

For fast iteration, `run-as` can write into the debuggable app without using the
picker:

```sh
adb push /path/to/MyGame /data/local/tmp/MyGame
adb shell run-as org.rsxp.mobile mkdir -p files/Games
adb shell run-as org.rsxp.mobile cp -R /data/local/tmp/MyGame files/Games/MyGame
adb shell am force-stop org.rsxp.mobile
adb shell am start -n org.rsxp.mobile/.MainActivity --es game "MyGame"
```

The folder name becomes the game ID. Remove an old copy first:

```sh
adb shell run-as org.rsxp.mobile rm -rf files/Games/MyGame
```

Saves are kept per game under `files/Games/.saves`. Uninstalling the app or
wiping the emulator removes imported games and saves.

## Logs and troubleshooting

```sh
adb logcat -s RSXP:D AndroidRuntime:E
```

Useful notes:

- The app targets API 28 intentionally so the Termux-patched Ruby can load its
  own standard-library extensions extracted from assets. Raising `targetSdk`
  requires moving those extensions into the APK's native-library directory.
- The UI uses native `ImageView` presentation for software frames, matching the
  existing Android shell. Native wgpu/Metal presentation remains iOS-only.
- `adb reverse` and networking permissions are not needed for local play.
- If Gradle cannot find the SDK, confirm you are inside `nix-shell
  shell-droid.nix` and that `ANDROID_HOME` points to the Nix SDK.
