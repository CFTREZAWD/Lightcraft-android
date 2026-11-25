//! The in-app updater: checks this repository's releases, downloads the newer APK (size and
//! SHA-256 checked) and hands it to Android's package installer (`LightcraftActivity.installApk`).
//!
//! The network runs on a worker thread; the UI thread only reads its answers. A check the user
//! asks for (Help ▸ Check for Updates…) always shows its outcome; the check at start shows only a
//! release found.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, OnceLock};

use lightcraft_ui_egui::i18n::tr;

use super::Activity;
use crate::update::{self, Release, Settings, Version};

/// The installer's answer, from `LightcraftActivity.nativeInstallStatus`.
pub(super) fn install_result() -> &'static Mutex<Option<Result<(), String>>> {
    static R: OnceLock<Mutex<Option<Result<(), String>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

enum State {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release),
    /// Downloaded and checked; waits for the installer (or for the permission to use it).
    Ready(Release, PathBuf),
    Installing(Release),
    Failed(String),
}

enum Msg {
    Checked(Result<Option<Release>, String>),
    Downloaded(Result<PathBuf, String>),
}

pub(super) struct Updater {
    settings: Settings,
    settings_path: PathBuf,
    downloads: PathBuf,
    state: State,
    rx: Option<Receiver<Msg>>,
    cancel: Arc<AtomicBool>,
    /// Bytes downloaded and expected.
    progress: Arc<Mutex<(u64, u64)>>,
    /// The window is open (a check the user asked for, or a release found).
    open: bool,
    /// The running check was asked for by the user (its outcome shows, whatever it is).
    manual: bool,
    /// Set by Help ▸ Check for Updates… (`Services::check_updates`).
    pub(super) requested: Arc<AtomicBool>,
    /// The check at start was considered.
    started: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// `GET` the release list (JSON) from GitHub's API.
fn fetch_releases(cancel: &AtomicBool) -> Result<serde_json::Value, String> {
    use lightcraft_fetch::http::{Limits, Url, get};
    let url = Url::parse(&format!("https://api.github.com/repos/{}/releases?per_page=30", update::REPO)).map_err(|e| e.to_string())?;
    let limits = Limits { connect: std::time::Duration::from_secs(15), stall: std::time::Duration::from_secs(30), cancel };
    let headers = [("Accept", "application/vnd.github+json".to_string()), ("X-GitHub-Api-Version", "2022-11-28".to_string())];
    let mut resp = get(&url, &headers, &limits).map_err(|e| e.to_string())?;
    if resp.status == 404 {
        // GitHub answers 404, not 403, for a private repository asked without signing in
        return Err(format!(
            "{} github.com/{} ({})",
            tr("No releases were found at"),
            update::REPO,
            tr("updates need the repository to be public")
        ));
    }
    if resp.status != 200 {
        return Err(format!("GitHub answered {}", resp.status));
    }
    let mut body = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = resp.read(&mut buf, &limits).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
        if body.len() > 8 * 1024 * 1024 {
            return Err("the release list is too large".into());
        }
    }
    serde_json::from_slice(&body).map_err(|e| format!("the release list couldn't be read: {e}"))
}

impl Updater {
    pub(super) fn new(settings_path: PathBuf, downloads: PathBuf) -> Self {
        Updater {
            settings: Settings::load(&settings_path),
            settings_path,
            downloads,
            state: State::Idle,
            rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new((0, 0))),
            open: false,
            manual: false,
            requested: Arc::new(AtomicBool::new(false)),
            started: false,
        }
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            log::warn!("{e}");
        }
    }

    fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading(_) | State::Installing(_))
    }

    fn check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.busy() {
            self.open |= manual;
            return;
        }
        self.manual = manual;
        self.open = manual;
        self.state = State::Checking;
        self.settings.last_check = now();
        self.save_settings();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let current = Version::parse(update::CURRENT).unwrap_or(Version { core: [0, 0, 0], pre: Vec::new() });
        let (betas, skipped) = (self.settings.betas, if manual { None } else { self.settings.skipped.clone() });
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("lc-update-check".into()).spawn(move || {
            let r = fetch_releases(&cancel).map(|list| update::newest(&list, &current, betas, skipped.as_deref()));
            let _ = tx.send(Msg::Checked(r));
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            self.state = State::Failed(e.to_string());
        }
    }

    fn download(&mut self, ctx: &egui::Context, release: Release) {
        let Some(sha) = release.sha256.clone() else {
            self.state = State::Failed(tr("This release has no checksum, so it isn't installed. Download it from its page instead.").into());
            return;
        };
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let progress = self.progress.clone();
        if let Ok(mut p) = progress.lock() {
            *p = (0, release.size);
        }
        let dir = self.downloads.clone();
        let _ = std::fs::remove_dir_all(&dir);
        let (url, size) = (release.apk_url.clone(), release.size);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("lc-update-download".into()).spawn(move || {
            let name = "update.apk";
            let spec = lightcraft_fetch::FileSpec { name, size: Some(size), sha256: Some(&sha), max: update::MAX_APK };
            let mut last = std::time::Instant::now();
            let r = lightcraft_fetch::download_url(&spec, &url, &dir, &lightcraft_fetch::Options::default(), &cancel, &mut |p| {
                if let Ok(mut g) = progress.lock() {
                    *g = (p.done, p.total.max(1));
                }
                if last.elapsed().as_millis() > 100 {
                    last = std::time::Instant::now();
                    ctx.request_repaint();
                }
            });
            let _ = tx.send(Msg::Downloaded(r.map(|()| dir.join(name)).map_err(|e| e.to_string())));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.state = State::Downloading(release),
            Err(e) => self.state = State::Failed(e.to_string()),
        }
    }

    fn install(&mut self, activity: &Activity, release: Release, path: PathBuf) {
        if !activity.can_install() {
            // Android asks once per app: Settings ▸ Install unknown apps ▸ LightCraft
            activity.open_install_settings();
            self.state = State::Ready(release, path);
            return;
        }
        if let Ok(mut r) = install_result().lock() {
            *r = None;
        }
        activity.install_apk(&path);
        self.state = State::Installing(release);
    }

    /// Per frame: the check at start, a requested check, the worker's and the installer's answers.
    pub(super) fn tick(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            if self.settings.due(now()) {
                self.check(ctx, false);
            }
        }
        if self.requested.swap(false, Ordering::Relaxed) {
            self.check(ctx, true);
        }
        let msg = self.rx.as_ref().and_then(|rx| rx.try_recv().ok());
        match msg {
            Some(Msg::Checked(Ok(Some(r)))) => {
                self.open = true;
                self.state = State::Available(r);
            }
            Some(Msg::Checked(Ok(None))) => self.state = State::UpToDate,
            Some(Msg::Checked(Err(e))) => {
                log::warn!("update check: {e}");
                self.state = State::Failed(e);
            }
            Some(Msg::Downloaded(r)) => {
                let st = std::mem::replace(&mut self.state, State::Idle);
                self.state = match (st, r) {
                    (State::Downloading(rel), Ok(path)) => {
                        self.open = true;
                        State::Ready(rel, path)
                    }
                    (_, Err(e)) => State::Failed(format!("{} {e}", tr("The download failed:"))),
                    (other, Ok(_)) => other,
                };
            }
            None => {}
        }
        if matches!(self.state, State::Installing(_))
            && let Some(r) = install_result().lock().ok().and_then(|mut g| g.take())
        {
            // (a successful install ends this process; an answer means it didn't happen)
            self.state = match r {
                Ok(()) => State::Idle,
                Err(e) => State::Failed(format!("{} {e}", tr("The update wasn't installed:"))),
            };
        }
        // an automatic check that found nothing (or failed) stays quiet
        if !self.open && !self.manual && matches!(self.state, State::UpToDate | State::Failed(_)) {
            self.state = State::Idle;
        }
    }

    pub(super) fn show(&mut self, ctx: &egui::Context, activity: &Activity) {
        if !self.open {
            return;
        }
        let mut open = true;
        let mut action: Option<Box<dyn FnOnce(&mut Updater)>> = None;
        let current = update::CURRENT;
        egui::Window::new(tr("LightCraft Updates"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(format!("{} {current}", tr("Installed version:")));
                ui.add_space(6.0);
                match &self.state {
                    State::Idle => {
                        ui.label(tr("Look for a newer LightCraft for Android."));
                    }
                    State::Checking => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(tr("Checking for updates…"));
                        });
                    }
                    State::UpToDate => {
                        ui.label(tr("LightCraft is up to date."));
                    }
                    State::Failed(e) => {
                        ui.label(egui::RichText::new(tr("The update check didn't work.")).strong());
                        ui.label(e.as_str());
                    }
                    State::Available(r) | State::Downloading(r) | State::Ready(r, _) | State::Installing(r) => {
                        ui.label(egui::RichText::new(format!("{} {}", tr("New version:"), r.version)).strong());
                        ui.label(format!("{} · {:.1} MB", r.name, r.size as f64 / 1_048_576.0));
                        if !r.notes.is_empty() {
                            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                ui.label(r.notes.as_str());
                            });
                        }
                    }
                }
                ui.add_space(8.0);
                match &self.state {
                    State::Downloading(_) => {
                        let (done, total) = self.progress.lock().map(|g| *g).unwrap_or((0, 1));
                        ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).show_percentage());
                        if ui.button(tr("Cancel")).clicked() {
                            self.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    State::Installing(_) => {
                        ui.label(tr("Android asks you to confirm the update. LightCraft restarts once it is installed."));
                    }
                    State::Ready(r, p) => {
                        if !activity.can_install() {
                            ui.label(tr("Allow LightCraft to install apps (in the screen that opens), then come back and tap Install."));
                        }
                        let (r, p) = (r.clone(), p.clone());
                        if ui.button(tr("Install")).clicked() {
                            let act = activity.clone();
                            action = Some(Box::new(move |u: &mut Updater| u.install(&act, r, p)));
                        }
                    }
                    State::Available(r) => {
                        let r2 = r.clone();
                        let page = r.page.clone();
                        let tag = r.tag.clone();
                        ui.horizontal(|ui| {
                            if ui.button(tr("Download and Install")).clicked() {
                                let ctx = ui.ctx().clone();
                                action = Some(Box::new(move |u: &mut Updater| u.download(&ctx, r2)));
                            }
                            if ui.button(tr("Later")).clicked() {
                                action = Some(Box::new(|u: &mut Updater| u.open = false));
                            }
                            if ui.button(tr("Skip This Version")).clicked() {
                                action = Some(Box::new(move |u: &mut Updater| {
                                    u.settings.skipped = Some(tag);
                                    u.save_settings();
                                    u.open = false;
                                    u.state = State::Idle;
                                }));
                            }
                        });
                        if !page.is_empty() && ui.link(tr("Release page")).clicked() {
                            let _ = activity.open_url(&page);
                        }
                    }
                    State::Idle | State::UpToDate | State::Failed(_) => {
                        if ui.button(tr("Check Again")).clicked() {
                            let ctx = ui.ctx().clone();
                            action = Some(Box::new(move |u: &mut Updater| u.check(&ctx, true)));
                        }
                    }
                    State::Checking => {}
                }
                ui.separator();
                let mut at_start = self.settings.check_at_start;
                let mut betas = self.settings.betas;
                let a = ui.checkbox(&mut at_start, tr("Check for updates when LightCraft starts")).changed();
                let b = ui.checkbox(&mut betas, tr("Include beta releases")).changed();
                if a || b {
                    self.settings.check_at_start = at_start;
                    self.settings.betas = betas;
                    self.save_settings();
                }
            });
        if let Some(f) = action {
            f(self);
        }
        if !open {
            self.open = false;
            if !self.busy() {
                self.cancel.store(true, Ordering::Relaxed);
                if !matches!(self.state, State::Ready(..)) {
                    self.state = State::Idle;
                }
            }
        }
    }
}
