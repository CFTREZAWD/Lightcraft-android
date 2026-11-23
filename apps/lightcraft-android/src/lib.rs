//! LightCraft for Android.
//!
//! The same engine and egui interface as the desktop app (`apps/lightcraft`), run as an Android
//! native activity (`android_main`). Build the APK with `tools/android/build-apk.sh`; see
//! `docs/android.md`.
//!
//! What differs from the desktop host:
//! - **Storage**: the library, settings and caches live in the app's private storage
//!   ([`paths`]). Photos come in through the system photo picker (no storage permission needed):
//!   the activity copies each chosen file into `<files>/Originals`, which the library then
//!   references. With the media permission granted, the Local panel also browses Pictures and
//!   DCIM in place. Exports are written to `Pictures/LightCraft`.
//! - **Pen input (S Pen and other active pens)** comes from the patched winit backend
//!   (`third_party/winit`, patch A1): pressure drives the brush's size and flow (Brush ▸ Pen
//!   Pressure), hovering shows the brush outline, the side button or the eraser end paints in the
//!   other mode (erase while adding, add while erasing: the Alt modifier), the palm is rejected
//!   while the pen is near, and every sample the pen reports is used.
//! - **Touch**: one finger works as a mouse, two fingers pinch-zoom and pan the loupe.
//! - **Settings** are saved whenever the app goes to the background: Android may end a
//!   backgrounded app without warning.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod paths;
pub mod prefs;

#[cfg(target_os = "android")]
mod android;

/// The pen state machine of the vendored winit (patch A1), built here so its tests run on the host.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../third_party/winit/src/platform_impl/android/stylus.rs"]
mod stylus;
