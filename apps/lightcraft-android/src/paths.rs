//! Where LightCraft keeps its files on Android.
//!
//! Android gives an app no `HOME` and no writable `/tmp`; the engine finds its folders through
//! environment variables (`LIGHTCRAFT_CONFIG_DIR`, `LIGHTCRAFT_LIBRARY`, `HOME`, `TMPDIR`), so the
//! host sets them to folders in the app's own storage before anything reads them.

use std::path::{Path, PathBuf};

/// The shared storage root every Android device mounts for the user (`/sdcard`).
pub const SHARED_STORAGE: &str = "/storage/emulated/0";

/// The app's folders, from the two Android gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    /// `Context.getFilesDir()`: private, kept until the app is uninstalled.
    pub files: PathBuf,
    /// `Context.getCacheDir()`: private, may be cleared by the system when space runs low.
    pub cache: PathBuf,
}

impl AppDirs {
    /// Settings, presets, models (`LIGHTCRAFT_CONFIG_DIR`).
    pub fn config(&self) -> PathBuf {
        self.files.join("config")
    }

    /// The library: catalog, previews, thumbnails (`LIGHTCRAFT_LIBRARY`).
    pub fn library(&self) -> PathBuf {
        self.files.join("LightCraft Library")
    }

    /// Photos chosen in the system picker are copied here (the picker hands out content URIs,
    /// not paths, and a library references its originals by path). Kept with the library.
    pub fn originals(&self) -> PathBuf {
        self.files.join("Originals")
    }

    /// Temporary files (`TMPDIR`, which `std::env::temp_dir` reads).
    pub fn temp(&self) -> PathBuf {
        self.cache.join("tmp")
    }

    /// The environment the engine reads its folders from, in the order to set it.
    pub fn environment(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("HOME", self.files.clone()),
            ("LIGHTCRAFT_CONFIG_DIR", self.config()),
            ("LIGHTCRAFT_LIBRARY", self.library()),
            ("XDG_CACHE_HOME", self.cache.clone()),
            ("TMPDIR", self.temp()),
        ]
    }

    /// The folders to create at start-up.
    pub fn to_create(&self) -> Vec<PathBuf> {
        vec![self.config(), self.originals(), self.temp()]
    }
}

/// Exports go to `Pictures/LightCraft` in shared storage, where the gallery shows them (Android
/// 11+ lets an app create files in the standard media folders without a permission).
pub fn export_dir() -> PathBuf {
    Path::new(SHARED_STORAGE).join("Pictures").join("LightCraft")
}

/// Where a file named `name` is saved: in [`export_dir`], with any folder part of the name dropped
/// (a suggested name is a name, never a way out of the folder).
pub fn save_path(name: &str) -> PathBuf {
    let base = Path::new(name).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty() && n != "..");
    export_dir().join(base.unwrap_or_else(|| "LightCraft export".into()))
}

/// The Local panel's places: the app's own copies first, then the shared photo folders (these
/// need the media permission to list).
pub fn local_places(dirs: &AppDirs) -> Vec<(String, String)> {
    let shared = Path::new(SHARED_STORAGE);
    vec![
        ("Imported".to_string(), dirs.originals().to_string_lossy().into_owned()),
        ("Camera".to_string(), shared.join("DCIM").to_string_lossy().into_owned()),
        ("Pictures".to_string(), shared.join("Pictures").to_string_lossy().into_owned()),
        ("Downloads".to_string(), shared.join("Download").to_string_lossy().into_owned()),
    ]
}

/// A file name for a copy in [`AppDirs::originals`] that doesn't replace one already there:
/// `name`, else `name (2)`, `name (3)`… before the extension. `exists` says whether a name is taken.
pub fn unique_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    let clean: String = name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    let clean = if clean.is_empty() || clean == "." || clean == ".." { "photo".to_string() } else { clean };
    if !exists(&clean) {
        return clean;
    }
    let (stem, ext) = match clean.rfind('.') {
        Some(i) if i > 0 => (&clean[..i], &clean[i..]),
        _ => (clean.as_str(), ""),
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|c| !exists(c)).unwrap_or(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> AppDirs {
        AppDirs { files: "/data/user/0/app/files".into(), cache: "/data/user/0/app/cache".into() }
    }

    #[test]
    fn everything_the_engine_writes_stays_in_the_apps_storage() {
        let d = dirs();
        for (_, p) in d.environment() {
            assert!(p.starts_with(&d.files) || p.starts_with(&d.cache), "{}", p.display());
        }
    }

    #[test]
    fn a_suggested_name_cannot_leave_the_export_folder() {
        for name in ["../../etc/passwd", "/abs/photo.jpg", "a/b/c.jpg", "..", ""] {
            assert_eq!(save_path(name).parent(), Some(export_dir().as_path()), "{name}");
        }
    }

    #[test]
    fn copies_never_replace_an_earlier_one() {
        let taken = ["IMG_1.jpg", "IMG_1 (2).jpg"];
        let name = unique_name("IMG_1.jpg", |n| taken.contains(&n));
        assert!(!taken.contains(&name.as_str()));
        assert!(name.ends_with(".jpg"));
        assert!(!unique_name("../x", |_| false).contains('/'));
    }
}
