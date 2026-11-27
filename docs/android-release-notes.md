**LightCraft for Android**: the LightCraft photo library and raw developer
([storytold/lightcraft](https://github.com/storytold/lightcraft) 0.21.1) on Android tablets and phones, with
Samsung S Pen and other active-pen support.

> **Beta.** Please report what happens on your device, with the output of `adb logcat -s LightCraft`
> if something fails.

### Install
Download `lightcraft-android-<version>-arm64.apk`, allow installs from your browser or file manager,
and open it, or run `adb install -r lightcraft-android-<version>-arm64.apk`. Needs Android 9+ on a
64-bit ARM device (any recent phone or tablet). Vulkan is used where there is one, otherwise OpenGL ES 3.

### Pen
- **Pressure** varies the brush size and flow (Brush ▸ Pen Pressure).
- **Hover** shows the brush outline before the pen touches the screen.
- **Side button held** (or the eraser end) paints in the other mode: erase while adding, add while erasing.
- **Palm rejection**: touches from your hand are ignored while the pen is near.
- **Every pen sample** is used (up to 240 Hz), so fast strokes keep their shape.
- Fingers: one finger acts as a mouse, two fingers pinch-zoom and pan.

### Updates
LightCraft checks for new releases at start (and in Help ▸ Check for Updates…), downloads the APK
with its checksum verified and lets Android install it after you confirm.

### Storage
Import opens Android's photo picker (the gallery; no permission needed); photos are copied into the app.
Raw files the gallery doesn't list are in the Local panel.
With the photos permission, the Local panel also browses Camera, Pictures and Downloads.
Exports go to `Pictures/LightCraft`.

### Known limits
arm64 only · AI masks run on the CPU (slow; a 3.4 GB download on first use; 8 GB of memory recommended) · no TalkBack ·
some predictive keyboards may not type correctly · one library, in the app's storage · desktop
layout (best on a tablet) · Chinese and Japanese text isn't rendered (fonts not bundled).
From 0.21.1-beta.2 on, releases are signed with one key and update each other in place; coming from
0.21.1-beta.1 (or a test build), uninstall it once first.

Details: [docs/android.md](https://github.com/CFTREZAWD/Lightcraft-android/blob/HEAD/docs/android.md).
