# PdfCraft for Android

The PdfCraft app (`crates/ui-egui`, the same interface as the desktop and web builds) running in
an Android [GameActivity](https://developer.android.com/games/agdk/game-activity) through eframe
and android-activity. Every tool, the look and the engine are shared; Android adds:

| Piece | Where | What it does |
|---|---|---|
| Rust entry point | `src/lib.rs` | `android_main`: starts eframe with the Activity, keeps settings in the app's data folder |
| JNI bridge | `src/jni_bridge.rs` | Files from intents into the app's inbox; the app's picker requests out to Kotlin and the answers back |
| File-picker stand-in | `crates/picker-android` | rfd's `AsyncFileDialog` API (rfd has no Android backend), used by `ui-egui` under the name `rfd` on Android |
| Clipboard stand-in | `crates/clipboard-android` | arboard's API for Android; image and file copy report "not available yet" (text copy and paste work through egui) |
| Activity | `android/app/src/main/java/.../MainActivity.kt` | System pickers (Storage Access Framework), "Open with" and share intents, writing saves back to the chosen documents |

## How files work

Android hands out `content://` URIs and the app works with paths, so:

- **Open** (and Insert, Combine, images…): the chosen documents are copied into the app's cache and
  opened from there. If the document can be written, **Save** writes back to it.
- **Save As / export one file**: you choose the document in the system picker; the app writes a
  staging file and each finished write is copied to that document.
- **Export to a folder**: you choose a folder; every file the app finishes is created there.
- **Open with PdfCraft / Share to PdfCraft**: the file's bytes go straight to the app.

## Building

Needs the Android SDK (platform 35, build-tools 35), NDK 27.2.12479018, a JDK 17+, and:

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

Then, from `apps/pdfcraft-android/android` (with `sdk.dir` in `local.properties`, or `ANDROID_HOME` set):

```sh
./gradlew assembleDebug                          # arm64 phones + the x86_64 emulator
./gradlew assembleDebug -Prust.abis=arm64-v8a    # phones only, half the build time
./gradlew installDebug                           # onto a connected device
```

Gradle runs `cargo ndk … build --release -p pdfcraft-android` first (`-Prust.profile=dev` for an
unoptimised Rust build), and takes the launcher icon from `assets/app-icon`. For Google Play,
build `./gradlew bundleRelease` with your signing config.

Logs: `adb logcat -s PdfCraft`.

## Known gaps

- The interface is the desktop one: it fits tablets and Chromebooks best. Phones work in
  landscape; a compact phone layout and touch gestures (pinch zoom, long press) come next.
- Copying images or files to the clipboard, printing, and system fonts beyond the bundled ones
  aren't wired up yet.
- The app's settings and crash recovery live in the app's private storage.
