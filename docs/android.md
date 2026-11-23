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
runs `.github/workflows/android-release.yml`: it builds the APK and publishes it with its SHA-256 as a
GitHub Release, with `docs/android-release-notes.md` as the notes. To sign with your own key, so that
releases install as updates of each other, add the secrets `ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD` and `ANDROID_KEY_ALIAS`; without them each run signs with a new debug key.

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

## Storage

- The library, settings and caches live in the app's private storage (removed with the app).
- **Import** uses the system file picker: no permission is needed, and each chosen photo is copied
  into the app's `Originals` folder, which the library references. Raw files that providers don't
  type as images are offered too (`application/octet-stream`).
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
