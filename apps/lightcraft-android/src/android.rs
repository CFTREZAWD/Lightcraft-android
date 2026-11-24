//! The Android host: `android_main`, the eframe app around [`LightcraftApp`], and the bridge to
//! `LightcraftActivity` (Java) for what native code can't do alone.
//!
//! This module is the crate's only `unsafe`: environment set-up before any thread reads it, and
//! the JNI handles Android passes in. Each block says why it holds.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use jni::JavaVM;
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::sys::jint;
use lightcraft_engine::Session;
use lightcraft_ui_egui::pick::{PickKind, PickRequest};
use lightcraft_ui_egui::{LightcraftApp, Services};
use winit::platform::android::activity::AndroidApp;

mod updater;

use crate::paths::{self, AppDirs};
use crate::prefs;

/// How long leaving the app waits for running renders.
const EXIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The extensions that mean "photos" in a pick request (then the picker offers images).
const PHOTO_HINTS: &[&str] = &["jpg", "jpeg", "dng", "png", "heic", "tif"];

/// Answers waiting for the Java picker, by request id; and the context to repaint when one comes.
struct Pending {
    next: i32,
    waiting: HashMap<i32, Sender<Vec<String>>>,
    errors: Vec<String>,
}

fn pending() -> &'static Mutex<Pending> {
    static P: OnceLock<Mutex<Pending>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Pending { next: 1, waiting: HashMap::new(), errors: Vec::new() }))
}

fn repaint_ctx() -> &'static OnceLock<egui::Context> {
    static C: OnceLock<egui::Context> = OnceLock::new();
    &C
}

/// The activity object and the VM, for calls into `LightcraftActivity`.
#[derive(Clone)]
struct Activity {
    app: AndroidApp,
}

impl Activity {
    /// Run `f` with a JNI environment attached to this thread and the activity object.
    fn with<R>(&self, f: impl FnOnce(&mut jni::JNIEnv<'_>, &JObject<'_>) -> jni::errors::Result<R>) -> Result<R, String> {
        // SAFETY: `vm_as_ptr` is the process's `JavaVM*`, valid for the life of the process.
        let vm = unsafe { JavaVM::from_raw(self.app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        // SAFETY: `activity_as_ptr` is the activity's global reference, valid while the app runs;
        // the `JObject` doesn't own it (it is never deleted through it).
        let activity = unsafe { JObject::from_raw(self.app.activity_as_ptr().cast()) };
        let r = f(&mut env, &activity);
        if env.exception_check().unwrap_or(false) {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
        }
        r.map_err(|e| e.to_string())
    }

    fn show_keyboard(&self, show: bool) {
        if let Err(e) = self.with(|env, act| env.call_method(act, "showKeyboard", "(Z)V", &[JValue::Bool(u8::from(show))]).map(|_| ())) {
            log::warn!("keyboard: {e}");
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.with(|env, act| {
            let s = env.new_string(url)?;
            env.call_method(act, "openUrl", "(Ljava/lang/String;)V", &[JValue::Object(&s)]).map(|_| ())
        })
    }

    /// Android lets this app install packages (Settings ▸ Install unknown apps).
    fn can_install(&self) -> bool {
        self.with(|env, act| env.call_method(act, "canInstall", "()Z", &[])?.z()).unwrap_or(false)
    }

    fn open_install_settings(&self) {
        let _ = self.with(|env, act| env.call_method(act, "openInstallSettings", "()V", &[]).map(|_| ()));
    }

    /// Hand a downloaded APK to the package installer (the answer comes back through
    /// `nativeInstallStatus`).
    fn install_apk(&self, path: &std::path::Path) {
        let path = path.to_string_lossy().into_owned();
        let r = self.with(|env, act| {
            let s = env.new_string(&path)?;
            env.call_method(act, "installApk", "(Ljava/lang/String;)V", &[JValue::Object(&s)]).map(|_| ())
        });
        if let Err(e) = r
            && let Ok(mut g) = updater::install_result().lock()
        {
            *g = Some(Err(e));
        }
    }

    fn request_media_access(&self) {
        let _ = self.with(|env, act| env.call_method(act, "requestMediaAccess", "()V", &[]).map(|_| ()));
    }

    /// Show the system picker; the copies' paths come back on the returned channel.
    fn pick(&self, mimes: &[&str], multiple: bool, dest: &std::path::Path) -> Result<Receiver<Vec<String>>, String> {
        let (tx, rx) = channel();
        let id = {
            let mut p = pending().lock().map_err(|_| "picker state poisoned".to_string())?;
            let id = p.next;
            p.next = p.next.wrapping_add(1).max(1);
            p.waiting.insert(id, tx);
            id
        };
        let dest = dest.to_string_lossy().into_owned();
        let started = self.with(|env, act| {
            let arr = env.new_object_array(mimes.len() as i32, "java/lang/String", JObject::null())?;
            for (i, m) in mimes.iter().enumerate() {
                let s = env.new_string(m)?;
                env.set_object_array_element(&arr, i as i32, s)?;
            }
            let d = env.new_string(&dest)?;
            env.call_method(
                act,
                "pick",
                "(I[Ljava/lang/String;ZLjava/lang/String;)V",
                &[JValue::Int(id), JValue::Object(&arr), JValue::Bool(u8::from(multiple)), JValue::Object(&d)],
            )
            .map(|_| ())
        });
        if let Err(e) = started {
            if let Ok(mut p) = pending().lock() {
                p.waiting.remove(&id);
            }
            return Err(format!("the file picker couldn't open: {e}"));
        }
        Ok(rx)
    }
}

/// `LightcraftActivity.nativePicked`: the picker closed and its files were copied in.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_lightcraft_LightcraftActivity_nativePicked<'l>(
    mut env: jni::JNIEnv<'l>,
    _class: JObject<'l>,
    id: jint,
    paths: JObjectArray<'l>,
    error: JString<'l>,
) {
    let mut out = Vec::new();
    let n = env.get_array_length(&paths).unwrap_or(0);
    for i in 0..n {
        if let Ok(obj) = env.get_object_array_element(&paths, i) {
            let s = JString::from(obj);
            if let Ok(p) = env.get_string(&s) {
                out.push(String::from(p));
            }
        }
    }
    let error: Option<String> = if error.is_null() { None } else { env.get_string(&error).ok().map(String::from) };
    if let Ok(mut p) = pending().lock() {
        if let Some(e) = error {
            p.errors.push(e);
        }
        if let Some(tx) = p.waiting.remove(&id) {
            let _ = tx.send(out);
        }
    }
    if let Some(ctx) = repaint_ctx().get() {
        ctx.request_repaint();
    }
}

/// `LightcraftActivity.nativeInstallStatus`: the package installer's answer (a success ends the
/// process as Android replaces the app, so it is rarely seen).
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_lightcraft_LightcraftActivity_nativeInstallStatus<'l>(
    mut env: jni::JNIEnv<'l>,
    _class: JObject<'l>,
    ok: jni::sys::jboolean,
    message: JString<'l>,
) {
    let msg: String = if message.is_null() { String::new() } else { env.get_string(&message).map(String::from).unwrap_or_default() };
    if let Ok(mut g) = updater::install_result().lock() {
        *g = Some(if ok != 0 { Ok(()) } else { Err(if msg.is_empty() { "cancelled".into() } else { msg }) });
    }
    if let Some(ctx) = repaint_ctx().get() {
        ctx.request_repaint();
    }
}

fn services(activity: &Activity, dirs: &AppDirs, update_requested: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Services {
    let picker_activity = activity.clone();
    let url_activity = activity.clone();
    let originals = dirs.originals();
    let picked = dirs.cache.join("picked");
    Services {
        local_places: Some(paths::local_places(dirs)),
        picker: Some(Box::new(move |req: PickRequest| match req.kind {
            PickKind::Files | PickKind::File => {
                let photos = req.filter.as_ref().is_some_and(|(_, exts)| exts.iter().any(|e| PHOTO_HINTS.contains(&e.to_ascii_lowercase().as_str())));
                // raws other than DNG are often typed application/octet-stream by the providers
                let (mimes, dest): (&[&str], _) =
                    if photos { (&["image/*", "application/octet-stream"], &originals) } else { (&["*/*"], &picked) };
                picker_activity.pick(mimes, req.kind == PickKind::Files, dest)
            }
            PickKind::Folder => {
                let (tx, rx) = channel();
                let dir = paths::export_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = tx.send(vec![dir.to_string_lossy().into_owned()]);
                Ok(rx)
            }
            PickKind::Save => {
                let (tx, rx) = channel();
                let path = paths::save_path(req.file_name.as_deref().unwrap_or_default());
                if let Some(d) = path.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = tx.send(vec![path.to_string_lossy().into_owned()]);
                Ok(rx)
            }
        })),
        open_url: Some(Box::new(move |url: &str| {
            if !url.starts_with("https://") {
                return Err("only https links are opened".into());
            }
            url_activity.open_url(url)
        })),
        // Help ▸ Check for Updates…: this app's releases, not the desktop packages
        check_updates: Some(Box::new(move || update_requested.store(true, std::sync::atomic::Ordering::Relaxed))),
        write_shared: Some(std::sync::Arc::new(lightcraft_engine::export::write_file)),
        write: Some(Box::new(lightcraft_engine::export::write_file)),
        write_new: Some(Box::new(lightcraft_engine::export::write_file_new)),
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        ..Default::default()
    }
}

/// The library at its place in the app's storage (a new one starts with the demo photos), or an
/// empty in-memory session and the reason when it can't be opened.
fn open_session(dirs: &AppDirs) -> (Session, Option<String>) {
    let fresh = || Session::new().with_fs().with_default_denoise_models().with_system_clock();
    let mut s = fresh();
    let dir = dirs.library();
    let first = !dir.exists();
    match s.open_library(&dir, first) {
        Ok(r) => {
            let replayed = r.replayed;
            log::info!("library {}: {} photos, {replayed} log records replayed", dir.display(), s.catalog.len());
        }
        Err(e) => {
            log::error!("can't open library {}: {e}", dir.display());
            return (fresh(), Some(format!("The library couldn't be opened ({e}). Changes in this session are not saved.")));
        }
    }
    s = s.with_default_lens_database().with_lens_auto_download();
    s.face_models_dir = lightcraft_engine::config::default_face_models_dir();
    s.slide_templates_dir = lightcraft_engine::config::config_dir().map(|d| d.join(lightcraft_engine::slide_template::DIR_NAME));
    (s, None)
}

/// Draws renders that stayed on the GPU (as the desktop host's `WindowTextures`).
struct WindowTextures {
    device: eframe::wgpu::Device,
    queue: eframe::wgpu::Queue,
    renderer: std::sync::Arc<egui::mutex::RwLock<eframe::egui_wgpu::Renderer>>,
}

impl lightcraft_ui_egui::render::ScreenTextures for WindowTextures {
    fn show(&mut self, frame: &lightcraft_engine::media::ScreenFrame, id: Option<egui::TextureId>) -> Option<egui::TextureId> {
        let view = frame.texture.create_view(&Default::default());
        let mut r = self.renderer.write();
        let filter = eframe::wgpu::FilterMode::Linear;
        Some(match id {
            Some(id) => {
                r.update_egui_texture_from_wgpu_texture(&self.device, &view, filter, id);
                id
            }
            None => r.register_native_texture(&self.device, &view, filter),
        })
    }

    fn free(&mut self, id: egui::TextureId) {
        self.renderer.write().free_texture(&id);
    }

    fn read(&self, frame: &lightcraft_engine::media::ScreenFrame) -> Option<egui::ColorImage> {
        lightcraft_engine::gpu::read_texture(&self.device, &self.queue, &frame.texture, frame.width, frame.height)
            .ok()
            .map(|img| egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], img.data.as_flattened()))
    }
}

fn adopt_window_device(rs: &eframe::egui_wgpu::RenderState, ctx: &egui::Context) -> WindowTextures {
    let (device, queue, info) = (rs.device.clone(), rs.queue.clone(), rs.adapter.get_info());
    let ctx = ctx.clone();
    lightcraft_engine::gpu::set_window_device_pending(true);
    let spawned = std::thread::Builder::new().name("lc-gpu-window".into()).spawn(move || {
        match lightcraft_engine::gpu::adopt_window_device(device, queue, info) {
            Ok(()) => log::info!("gpu: rendering on the window's device"),
            Err(e) => log::info!("gpu: renders come back as pixels: {e}"),
        }
        ctx.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("gpu: {e}");
        lightcraft_engine::gpu::set_window_device_pending(false);
    }
    WindowTextures { device: rs.device.clone(), queue: rs.queue.clone(), renderer: rs.renderer.clone() }
}

struct App {
    app: LightcraftApp,
    prefs: prefs::Writer,
    textures: Option<WindowTextures>,
    activity: Activity,
    /// The soft keyboard is up (a text field has the focus).
    keyboard: bool,
    /// The app had the focus last frame (leaving it saves).
    focused: bool,
    updater: updater::Updater,
}

impl App {
    fn save(&mut self) {
        if let Err(e) = self.prefs.save(&self.app.ui) {
            log::error!("{e}");
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgb(12, 12, 12).to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
        // picker problems reported by the activity
        if let Ok(mut p) = pending().lock() {
            self.app.notices.append(&mut p.errors);
        }
        // Android may end a backgrounded app without notice: save as it leaves the foreground
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.focused && !focused {
            self.save();
        }
        self.focused = focused;
        if let Err(e) = self.prefs.tick(&self.app.ui, self.app.tick_now()) {
            log::warn!("{e}");
        }
        self.updater.tick(ctx);
        // the soft keyboard follows the text focus
        let wants = ctx.egui_wants_keyboard_input();
        if wants != self.keyboard {
            self.keyboard = wants;
            self.activity.show_keyboard(wants);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.perf.host_frame_ms = frame.info().cpu_usage.map(|s| f64::from(s) * 1000.0);
        if self.textures.is_some()
            && lightcraft_engine::gpu::on_window_device()
            && let Some(t) = self.textures.take()
        {
            self.app.renderer.set_screen_textures(Box::new(t));
        }
        self.app.ui(ui);
        self.updater.show(ui.ctx(), &self.activity);
    }

    fn on_exit(&mut self) {
        self.save();
        if let Err(e) = self.app.session.close_library() {
            log::error!("saving the library failed: {e}");
        }
        if !self.app.shutdown(EXIT_WAIT) {
            log::warn!("quit: background work was still running after {EXIT_WAIT:?}");
        }
    }
}

/// Point the engine at the app's folders. Runs first in `android_main`, before any other thread
/// of ours exists.
fn set_up_environment(dirs: &AppDirs) {
    for d in dirs.to_create() {
        if let Err(e) = std::fs::create_dir_all(&d) {
            log::warn!("can't create {}: {e}", d.display());
        }
    }
    for (k, v) in dirs.environment() {
        // SAFETY: called at the start of `android_main`, before this process starts any thread
        // that reads the environment (the Java threads don't read the C environment).
        unsafe { std::env::set_var(k, v) };
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("LightCraft"));
    let files = app.internal_data_path().unwrap_or_else(|| "/data/local/tmp/lightcraft".into());
    let cache = files.parent().map(|p| p.join("cache")).unwrap_or_else(|| files.join("cache"));
    let dirs = AppDirs { files, cache };
    set_up_environment(&dirs);
    log::info!("LightCraft {} for Android, data in {}", env!("CARGO_PKG_VERSION"), dirs.files.display());
    lightcraft_engine::guard::install_hook(dirs.temp().join("lightcraft-panic.log"));

    let prefs_path = dirs.config().join("ui.json");
    let (saved, prefs_warning) = prefs::load(&prefs_path);
    if let Some(ui) = &saved {
        lightcraft_ui_egui::i18n::set_language(ui.language);
    }
    lightcraft_engine::gpu::set_enabled(saved.as_ref().is_none_or(|u| u.settings.gpu));

    let activity = Activity { app: app.clone() };
    activity.request_media_access();
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_title("LightCraft"),
        ..Default::default()
    };
    let result = eframe::run_native(
        "LightCraft",
        options,
        Box::new(move |cc| {
            let _ = repaint_ctx().set(cc.egui_ctx.clone());
            let (session, problem) = open_session(&dirs);
            let updater = updater::Updater::new(dirs.config().join("android-updates.json"), dirs.cache.join("update"));
            let mut lc = LightcraftApp::new(session, services(&activity, &dirs, updater.requested.clone()));
            if let Some(ui) = saved {
                lc.ui = ui;
            }
            lightcraft_ui_egui::i18n::set_language(lc.ui.language);
            lc.notices.extend(prefs_warning);
            lc.notices.extend(problem);
            lc.gpu_notice = true;
            lc.disk_space_notice = true;
            if lc.session.catalog.is_empty() {
                lightcraft_ui_egui::panels::empty_library::open_on_empty_library(&mut lc);
            }
            let prefs = prefs::Writer::new(Some(prefs_path), &lc.ui);
            let gpu_on = lc.ui.settings.gpu;
            let textures = cc.wgpu_render_state.as_ref().filter(|_| gpu_on).map(|rs| adopt_window_device(rs, &cc.egui_ctx));
            Ok(Box::new(App { app: lc, prefs, textures, activity, keyboard: false, focused: true, updater }))
        }),
    );
    if let Err(e) = result {
        log::error!("LightCraft stopped: {e}");
    }
}
