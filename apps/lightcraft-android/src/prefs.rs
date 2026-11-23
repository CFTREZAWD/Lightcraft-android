//! `<config>/ui.json`: the interface state and app settings, as the desktop app keeps them.
//!
//! Android may end an app in the background without running any exit code, so the host saves
//! whenever the app leaves the foreground, and every few seconds after a change.

use std::path::{Path, PathBuf};

use lightcraft_ui_egui::UiState;

/// The saved settings, or `None` with a message for the user when the file is there but can't be
/// used (it is then set aside as `ui.json.damaged`, so the next save doesn't lose it silently).
pub fn load(path: &Path) -> (Option<UiState>, Option<String>) {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, None),
        Err(e) => return (None, Some(format!("The app settings couldn't be read ({e}); defaults are used."))),
    };
    match decode(&bytes) {
        Ok(ui) => (Some(ui.sanitized()), None),
        Err(e) => {
            let _ = std::fs::rename(path, path.with_extension("json.damaged"));
            (None, Some(format!("The app settings were damaged ({e}) and were set aside; defaults are used.")))
        }
    }
}

fn decode(bytes: &[u8]) -> Result<UiState, String> {
    let mut saved: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let fields = saved.as_object_mut().ok_or("settings must be an object")?;
    lightcraft_ui_egui::docking::drop_outdated(fields);
    // a panel layout this version can't use starts again from the default
    let bad_layout = fields.get("docking").is_some_and(|v| match serde_json::from_value::<Option<lightcraft_ui_egui::docking::Workspace>>(v.clone()) {
        Ok(Some(w)) => lightcraft_ui_egui::docking::validate(&w).is_err(),
        Ok(None) => false,
        Err(_) => true,
    });
    if bad_layout {
        fields.remove("docking");
    }
    lightcraft_ui_egui::startup::migrate_legacy_seen_version(fields);
    serde_json::from_value(saved).map_err(|e| e.to_string())
}

/// Writes `ui.json` when it changed: atomically (a temporary file, synced, renamed over it).
#[derive(Debug, Default)]
pub struct Writer {
    pub path: Option<PathBuf>,
    written: Vec<u8>,
    /// When the settings were last compared (seconds on the app's clock).
    checked: f64,
}

impl Writer {
    pub fn new(path: Option<PathBuf>, ui: &UiState) -> Self {
        Writer { path, written: serde_json::to_vec_pretty(ui).unwrap_or_default(), checked: 0.0 }
    }

    /// Save now if anything changed.
    pub fn save(&mut self, ui: &UiState) -> Result<(), String> {
        let Some(path) = self.path.clone() else { return Ok(()) };
        if let Some(w) = &ui.docking {
            lightcraft_ui_egui::docking::validate(w)?;
        }
        let bytes = serde_json::to_vec_pretty(ui).map_err(|e| e.to_string())?;
        if bytes == self.written {
            return Ok(());
        }
        write_atomic(&path, &bytes).map_err(|e| format!("saving the app settings failed: {e}"))?;
        self.written = bytes;
        Ok(())
    }

    /// Save at most every three seconds (`now` on the app's clock).
    pub fn tick(&mut self, ui: &UiState, now: f64) -> Result<(), String> {
        if now - self.checked < 3.0 {
            return Ok(());
        }
        self.checked = now;
        self.save(ui)
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-android-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saved_settings_come_back() {
        let dir = temp("roundtrip");
        let path = dir.join("ui.json");
        let mut ui = UiState::default();
        ui.brush_pressure = !ui.brush_pressure;
        let mut w = Writer::new(Some(path.clone()), &UiState::default());
        w.save(&ui).unwrap();
        let (back, warning) = load(&path);
        assert!(warning.is_none());
        assert_eq!(back.map(|b| b.brush_pressure), Some(ui.brush_pressure));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_set_aside_not_lost() {
        let dir = temp("damaged");
        let path = dir.join("ui.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let (ui, warning) = load(&path);
        assert!(ui.is_none() && warning.is_some());
        assert!(path.with_extension("json.damaged").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
