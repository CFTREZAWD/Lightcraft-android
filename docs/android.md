# LightCraft for Android

This fork runs LightCraft on Android tablets and phones (arm64, Android 9+), with full support for
active pens such as the Samsung **S Pen**. It is the same engine and the same egui interface as the
desktop app; `apps/lightcraft-android` is the Android host.

## Build

Requirements: Rust (the workspace's `rust-version`) with `rustup target add aarch64-linux-android`,
the Android NDK (r27 or later), the SDK build-tools and a platform (34+), and a JDK.

```sh
ANDROID_NDK_HOME=~/Android/Sdk/ndk/28.2.13676358 ANDROID_HOME=~/Android/Sdk tools/android/build-apk.sh
adb install -r target/android/lightcraft-*-arm64.apk
```

`--debug` builds the unoptimized library (much slower renders). The APK is signed with a local
debug key; re-sign it with your own for distribution. Logs: `adb logcat -s LightCraft`.

## Releases

Pushing a tag `android-v<version>` (e.g. `android-v0.21.1-beta.1`; a suffix makes it a pre-release)
(or running `.github/workflows/android-release.yml` by hand with the tag name; the tag is then made on
the branch's commit) runs the workflow: it builds the APK and publishes it with its SHA-256 as a
GitHub Release, with `docs/android-release-notes.md` as the notes. To sign with your own key, so that
releases install as updates of each other, add the secrets `ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS`; without them each run signs with a new debug key.

## Updates

LightCraft for Android updates itself from this repository's releases:

- At start (at most about once a day; *Check for updates when LightCraft starts*) and from
  Help ▸ Check for Updates…, it reads the GitHub Releases tagged `android-v*` and offers the newest
  one above the installed version. Betas are offered unless *Include beta releases* is off; a
  version you chose to skip isn't offered again by the check at start.
- **Download and Install** fetches the `-arm64.apk` asset with its size and SHA-256 checked (the
  digest GitHub reports for the asset; a release without one isn't installed), then hands it to
  Android's package installer, which asks you to confirm. The first time, Android asks you to
  allow LightCraft to install apps.
- Android installs an update only when it is signed with the same key as the installed app. So
  releases must be signed with one key: run `tools/android/make-release-key.sh` once and store
  the three values it prints as the repository secrets `ANDROID_KEYSTORE_BASE64`,
  `ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS`. Without them every release gets a new debug
  key and the update is refused ("signed with another key"): uninstall, then install the new one.
  `0.21.1-beta.1` was signed that way and has no updater: install the next release by hand once.
- On Samsung devices, **Auto Blocker** (Settings ▸ Security and privacy) blocks every install outside
  the stores, updates included: the install then reads as cancelled. Turn it off to update.
- The repository must be **public**: GitHub answers 404 to an app asking about a private one, and
  an app can't safely carry a token (anyone with the APK could read it).
- The version the updater compares is the release's (`LIGHTCRAFT_ANDROID_VERSION`, from its tag);
  `versionCode` grows with every release, betas included. A build for a fork looks for that
  fork's releases (`LIGHTCRAFT_UPDATE_REPO`, set by the workflow).

## S Pen and active pens

The pen support lives in the Android backend of winit, vendored in `third_party/winit` (patch
**A1**, `src/platform_impl/android/stylus.rs`; listed in `third_party/winit/PATCHES.md`):

| Pen action | In LightCraft |
|---|---|
| Pressure | The mask and lens-blur brushes vary their size and flow (Brush ▸ Pen Pressure). Fingers report no pressure, so they paint at full strength, like a mouse. |
| Hover | The brush outline follows the pen before it touches the screen. |
| Side button held | The brush paints in the other mode: erases while adding, adds while erasing (the Alt modifier on the desktop). |
| Eraser end (pens that have one) | Same as the side button. |
| Palm on the screen | Ignored while the pen hovers or touches; a finger already down when the pen lands is ended; touches Android itself flags as palms are ended. |
| Fast strokes | Every sample the pen reports (up to 240 Hz on an S Pen) is used, not only one per frame, so curves keep their shape. |

Fingers still work everywhere: one finger acts as a mouse, two fingers pinch-zoom and pan the loupe.

The state machine works on plain data, so its tests run on any machine:
`cargo test -p lightcraft-android` builds `stylus.rs` into the host test binary.

## Screen

The app runs full screen: Android's status bar (clock, notifications) and navigation bar are
hidden, since they would cover the top bar's menus and the side panels. Swipe from the top or
bottom edge to show them for a moment.

## Storage

- The library, settings and caches live in the app's private storage (removed with the app).
- **Import** opens Android's photo picker (your gallery: Android 13+, and Android 11-12 with the
  Google Play photo picker update); elsewhere, the system file picker. Neither needs a permission:
  each chosen photo is copied into the app's `Originals` folder, which the library references.
  The photo picker shows what the gallery shows: raw files it doesn't list (often CR3, NEF, ARW…)
  are reached through the Local panel, and Android removes the GPS location from photos chosen
  there (a privacy rule of the photo picker).
- With the photos permission (asked once at first launch), the **Local** panel also browses
  Camera (DCIM), Pictures and Downloads in place.
- **Exports** and other saved files go to `Pictures/LightCraft`, where the gallery shows them.
- Settings are saved every few seconds after a change and whenever the app leaves the foreground,
  since Android may close a backgrounded app without warning.

## Known limits

- arm64 only (`arm64-v8a`); 32-bit and x86_64 aren't built.
- AI masks with SAM 3 (Object / Describe) are left out, as in the web build: the model is several
  gigabytes. Denoise is included.
- Text input uses the system keyboard through the native activity's key events: plain text works;
  some keyboards' composing (predictive) input may not.
- Choosing a library folder (Settings ▸ Open Library…) and the dialogs' own Browse… buttons aren't
  available: there is one library, in the app's storage.
- Accessibility (TalkBack) isn't connected: eframe offers it only with the GameActivity backend.
- The layout is the desktop one: comfortable on a tablet (Galaxy Tab S series), dense on a phone.
