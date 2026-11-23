**LightCraft for Android**: the LightCraft photo library and raw developer
([storytold/lightcraft](https://github.com/storytold/lightcraft) 0.21.1) on Android tablets and phones, with
Samsung S Pen and other active-pen support.

> **Beta.** This build compiles, packages and signs cleanly, but it hasn't been tried on a device yet.
> Please report what happens on yours, with the output of `adb logcat -s LightCraft` if it fails.

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

### Storage
Import uses the system file picker (no permission needed); photos are copied into the app.
With the photos permission, the Local panel also browses Camera, Pictures and Downloads.
Exports go to `Pictures/LightCraft`.

### Known limits
arm64 only · no SAM 3 AI masks (the model is several GB; denoise is included) · no TalkBack ·
some predictive keyboards may not type correctly · one library, in the app's storage · desktop
layout (best on a tablet) · Chinese and Japanese text isn't rendered (fonts not bundled).
The APK is signed with a debug key, so a future release signed with another key needs an uninstall first.

Details: [docs/android.md](https://github.com/CFTREZAWD/Lightcraft-android/blob/HEAD/docs/android.md).
