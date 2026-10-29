//! The OpenCeres window: a slim toolbar, the image stage (almost all of the
//! window), an optional report side panel and a slim status bar.

pub(crate) mod settings_ui;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{
    self, Align, Align2, Button, Color32, CursorIcon, Event, FontId, Frame, Id, Key, Label, Layout,
    Margin, Order, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind, TextFormat,
    ThemePreference, UiBuilder, Vec2, pos2, text::LayoutJob, vec2,
};

use crate::capture::{self, Capture};
use crate::format;
use crate::gpu::{Draw, Gpu};
use crate::icons::{self, Icon};
use crate::known::{self, Catalogue, Download, Fetched, Freshness, Lookup, LookupError};
use crate::load::{self, Frame as ImageFrame, ImageSet};
use crate::prefs::{GRID_SIZES, KnownSource, Prefs, SPEEDS, ThemeChoice};
use crate::settings::Category;
use crate::theme::{self, S1, S2, S3, S4, S5, palette};
use crate::update;
use openceres_core::known::KnownObject;

const MAX_SCALE: f32 = 32.0;
/// Pointer movement (points) that turns a click into a drag.
const CLICK_SLOP: f32 = 4.0;
/// Seconds a blink switch may happen before it is due.
const SWITCH_EARLY: f64 = 0.03;
/// Frames to draw (after loading) before applying a capture's state, then
/// frames and seconds to wait (for fade-ins) before taking the screenshot.
const CAPTURE_SETTLE: u32 = 2;
const CAPTURE_SHOOT: u32 = 6;
const CAPTURE_WAIT: f64 = 0.6;
/// Seconds between automatic update and catalogue checks.
const DAY: u64 = 86_400;

struct Set {
    name: String,
    /// Frames in time order, without their pixels (those live on the GPU).
    frames: Vec<ImageFrame>,
    summary: String,
}

struct Loading {
    message: String,
    rx: Receiver<Result<ImageSet, String>>,
}

/// Known objects for the open set: being looked up, found, or why not.
enum KnownState {
    Pending(Receiver<Result<Lookup, LookupError>>),
    Done(KnownField),
    /// The offline source is chosen but the catalogue is not downloaded.
    NoCatalogue,
    Failed(String),
}

struct KnownField {
    /// Objects inside the image on at least one frame, brightest first.
    objects: Vec<Placed>,
    source: KnownSource,
    at: String,
    catalogue: Option<String>,
}

struct Placed {
    object: KnownObject,
    /// Position on each frame (0-based FITS pixels), `None` when outside.
    at: Vec<Option<(f64, f64)>>,
}

#[derive(Default)]
struct View {
    /// Screen points per image pixel.
    scale: f32,
    /// Image top-left relative to the stage's top-left, in points.
    offset: Vec2,
    stage: Vec2,
    fitted: bool,
    /// Still as fitted: not zoomed or panned since, so a new stage size
    /// (side panel, window) fits again.
    auto_fit: bool,
    drag: Option<Drag>,
}

/// The app update check (see [`crate::update`]).
#[derive(Default)]
struct Updates {
    pending: Option<Receiver<Result<update::Check, String>>>,
    /// The pending check was asked for in Settings, so errors are shown.
    manual: bool,
    /// Set once the automatic check has been considered this session.
    auto_done: bool,
    found: Option<update::Release>,
    error: Option<String>,
    /// Unix time of this session's last successful check.
    checked_at: Option<u64>,
    downloading: Option<Receiver<Result<PathBuf, String>>>,
    downloaded: Option<PathBuf>,
    download_error: Option<String>,
}

struct Drag {
    start: Pos2,
    offset: Vec2,
    moved: bool,
}

pub struct App {
    /// Defaults and the side panel state, saved when they change.
    prefs: Prefs,
    saved: Prefs,
    theme: Option<ThemeChoice>,
    gpu: Gpu,
    set: Option<Set>,
    index: usize,
    playing: bool,
    /// `egui` input time at which the next blink switch is due.
    next_switch: f64,
    // Session state, started from the defaults in `prefs`.
    blink_seconds: f32,
    show_stars: bool,
    invert: bool,
    grid_on: bool,
    grid_size: usize,
    /// Marked report boxes as (row, column), 1-based from the top-left.
    marks: Vec<(usize, usize)>,
    copied: bool,
    /// Known objects shown (session toggle, K).
    known_on: bool,
    known: Option<KnownState>,
    /// Known object the view was last centred on (index into the list).
    known_focus: Option<usize>,
    download: Option<Download>,
    download_error: Option<String>,
    /// e.g. "Already up to date", after a download that fetched nothing.
    download_note: Option<String>,
    /// MPCORB freshness check (HEAD only) and whether Settings asked for it.
    mpcorb_check: Option<(Receiver<Result<Freshness, String>>, bool)>,
    /// Result of the last check asked for in Settings.
    mpcorb_fresh: Option<Result<Freshness, String>>,
    /// The background check found a newer catalogue (hint in the panel).
    mpcorb_newer: bool,
    mpcorb_auto_done: bool,
    updates: Updates,
    /// What Settings shows about the offline catalogue; `None` = read again.
    catalogue: Option<Catalogue>,
    loading: Option<Loading>,
    error: Option<String>,
    view: View,
    /// Cursor position in 0-based FITS pixels (y up).
    hover: Option<(f64, f64)>,
    settings_open: bool,
    /// Page Settings shows (and reopens on).
    settings_page: Category,
    settings_search: String,
    /// "Reset all" was clicked once and waits for confirmation.
    confirm_reset: bool,
    help_open: bool,
    capture: Option<Capture>,
}

/// Key caps and what they do, for the help overlay and the Settings window.
fn shortcuts() -> Vec<(Vec<String>, &'static str)> {
    let cmd = if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl+"
    };
    let keys = |k: &[&str]| k.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    vec![
        (keys(&["Space"]), "Play / pause blinking"),
        (keys(&["←", "→"]), "Previous / next frame"),
        (keys(&["1", "–", "4"]), "Show one frame"),
        (keys(&[",", "."]), "Blink slower / faster (or [ ])"),
        (keys(&["+", "−", "0"]), "Zoom in / out, fit"),
        (keys(&["S"]), "Detected stars"),
        (keys(&["I"]), "Invert"),
        (keys(&["G"]), "Report grid (click a box to mark it)"),
        (keys(&["K"]), "Known objects in the field"),
        (keys(&["Tab"]), "Side panel (or \\)"),
        (vec!["O".into(), format!("{cmd}O")], "Open an image set"),
        (vec![format!("{cmd},")], "Settings"),
        (keys(&["?"]), "This list"),
    ]
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Vec<String>,
        capture: Option<Capture>,
    ) -> App {
        theme::install(&cc.egui_ctx);
        // Captures start from the defaults so they look the same everywhere.
        let mut prefs = if capture.is_some() {
            Prefs::default()
        } else {
            Prefs::load()
        };
        if let Some(c) = &capture {
            match c.state.theme {
                Some(egui::Theme::Light) => prefs.theme = ThemeChoice::Light,
                Some(egui::Theme::Dark) => prefs.theme = ThemeChoice::Dark,
                None => {}
            }
            prefs.panel_open |= c.state.panel;
        }
        let mut app = App {
            prefs,
            saved: prefs,
            theme: None,
            gpu: Gpu::default(),
            set: None,
            index: 0,
            playing: true,
            next_switch: 0.0,
            blink_seconds: prefs.blink_seconds,
            show_stars: prefs.show_stars,
            invert: prefs.invert,
            grid_on: false,
            grid_size: prefs.grid_size,
            marks: Vec::new(),
            copied: false,
            known_on: false,
            known: None,
            known_focus: None,
            download: None,
            download_error: None,
            download_note: None,
            mpcorb_check: None,
            mpcorb_fresh: None,
            mpcorb_newer: false,
            mpcorb_auto_done: false,
            updates: Updates::default(),
            catalogue: None,
            loading: None,
            error: None,
            view: View::default(),
            hover: None,
            settings_open: false,
            settings_page: Category::General,
            settings_search: String::new(),
            confirm_reset: false,
            help_open: false,
            capture,
        };
        if !paths.is_empty() {
            app.open(&cc.egui_ctx, paths);
        }
        app
    }

    fn period(&self) -> f64 {
        f64::from(self.blink_seconds)
    }

    fn frame_count(&self) -> usize {
        self.set.as_ref().map_or(0, |s| s.frames.len())
    }

    fn current(&self) -> Option<&ImageFrame> {
        self.set.as_ref().and_then(|s| s.frames.get(self.index))
    }

    fn open(&mut self, ctx: &egui::Context, paths: Vec<String>) {
        if self.loading.is_some() {
            return;
        }
        let any = !paths.is_empty();
        let files: Vec<String> = paths
            .into_iter()
            .filter(|p| load::has_extension(p, &load::OPENABLE_EXTENSIONS))
            .collect();
        if files.is_empty() {
            if any {
                self.error = Some(load::NOT_OPENABLE.into());
            }
            return;
        }
        let message = if files.iter().any(|p| load::has_extension(p, &["zip"])) {
            "Unpacking and loading…".to_owned()
        } else {
            let n = files.len();
            format!("Loading {n} image{}…", if n > 1 { "s" } else { "" })
        };
        self.error = None;
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load::open(&files));
            ctx.request_repaint();
        });
        self.loading = Some(Loading { message, rx });
    }

    fn choose(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if self.loading.is_some() {
            return;
        }
        let picked = rfd::FileDialog::new()
            .set_title("Open image set")
            .add_filter("FITS images or zip", &load::OPENABLE_EXTENSIONS)
            .set_parent(frame)
            .pick_files();
        if let Some(paths) = picked {
            let paths = paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            self.open(ctx, paths);
        }
    }

    fn poll_loading(&mut self, now: f64) {
        let Some(loading) = &self.loading else { return };
        let result = match loading.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("image loader crashed".into()),
        };
        self.loading = None;
        match result {
            Ok(ImageSet { name, mut frames }) => {
                let pixels = frames
                    .iter_mut()
                    .map(|f| (f.width, f.height, std::mem::take(&mut f.gray)))
                    .collect();
                self.gpu.set_frames(pixels);
                let summary = summary(&frames);
                self.set = Some(Set {
                    name,
                    frames,
                    summary,
                });
                self.marks.clear();
                self.copied = false;
                self.known = None;
                self.known_focus = None;
                self.index = 0;
                self.playing = self.frame_count() > 1;
                self.next_switch = now + self.period();
                self.view = View::default();
                self.hover = None;
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn step(&mut self, delta: isize) {
        let n = self.frame_count();
        if n == 0 {
            return;
        }
        self.playing = false;
        self.index = (self.index as isize + delta).rem_euclid(n as isize) as usize;
    }

    fn toggle_play(&mut self, now: f64) {
        if self.set.is_some() {
            self.playing = !self.playing;
            self.next_switch = now + self.period();
        }
    }

    /// `slower` > 0 moves to the next slower speed, < 0 to the next faster one.
    fn change_speed(&mut self, slower: isize, now: f64) {
        let at = SPEEDS
            .iter()
            .position(|&s| s >= self.blink_seconds)
            .unwrap_or(SPEEDS.len() - 1);
        let i = (at as isize + slower.signum()).clamp(0, SPEEDS.len() as isize - 1);
        self.set_speed(SPEEDS[i as usize], now);
    }

    fn set_speed(&mut self, seconds: f32, now: f64) {
        self.blink_seconds = seconds;
        self.next_switch = now + self.period();
    }

    /// Turning the grid on is an explicit request to report, so it also
    /// opens the side panel that holds the report.
    fn set_grid(&mut self, on: bool) {
        self.grid_on = on;
        if on {
            self.prefs.panel_open = true;
        }
    }

    fn set_grid_size(&mut self, n: usize) {
        if n != self.grid_size {
            self.marks.clear(); // box labels mean different areas now
            self.copied = false;
        }
        self.grid_size = n;
    }

    fn report(&self) -> String {
        let name = self
            .set
            .as_ref()
            .map(|s| s.name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("Image set");
        report_line(name, &self.marks)
    }

    fn set_known(&mut self, on: bool) {
        self.known_on = on;
        if on {
            self.prefs.panel_open = true;
        }
    }

    /// Starts the lookup for the open set when known objects are on and it
    /// has not been done yet (once per set and source).
    fn ensure_known(&mut self, ctx: &egui::Context) {
        if !self.known_on || self.known.is_some() {
            return;
        }
        let Some(set) = &self.set else { return };
        self.known = Some(match known::field(&set.frames) {
            Ok(field) => KnownState::Pending(known::start(ctx, self.prefs.known_source, field)),
            Err(e) => KnownState::Failed(e),
        });
    }

    fn poll_known(&mut self) {
        let Some(KnownState::Pending(rx)) = &self.known else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(LookupError::Message(
                "The lookup stopped unexpectedly.".into(),
            )),
        };
        self.known = Some(match (result, &self.set) {
            (Ok(lookup), Some(set)) => KnownState::Done(place(lookup, &set.frames)),
            (Ok(_), None) => return self.known = None,
            (Err(LookupError::NoCatalogue), _) => KnownState::NoCatalogue,
            (Err(LookupError::Message(e)), _) => KnownState::Failed(e),
        });
    }

    fn poll_download(&mut self) {
        let Some(d) = &self.download else { return };
        let result = match d.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The download stopped unexpectedly.".into()),
        };
        self.download = None;
        self.catalogue = None;
        match result {
            Ok(fetched) => {
                self.download_error = None;
                self.mpcorb_newer = false;
                self.mpcorb_fresh = None;
                self.prefs.mpcorb_last_check = format::unix_now();
                if let Fetched::AlreadyCurrent = fetched {
                    self.download_note =
                        Some("Already up to date — nothing was downloaded.".into());
                } else if self.prefs.known_source == KnownSource::Mpcorb {
                    self.known = None; // look up again with the new catalogue
                }
            }
            Err(e) => self.download_error = Some(e),
        }
    }

    /// Asks the MPC whether the catalogue is current (HEAD only). Silent
    /// checks only set the panel hint; Settings shows the result.
    fn start_mpcorb_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.mpcorb_check.is_none() {
            self.mpcorb_check = Some((known::start_check(ctx), manual));
            if manual {
                self.mpcorb_fresh = None;
            }
        }
    }

    fn poll_mpcorb_check(&mut self) {
        let Some((rx, manual)) = &self.mpcorb_check else {
            return;
        };
        let manual = *manual;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The check stopped unexpectedly.".into()),
        };
        self.mpcorb_check = None;
        match &result {
            // Remembered only when current, so a newer catalogue is
            // pointed out again on the next start.
            Ok(Freshness::Current(_)) => {
                self.prefs.mpcorb_last_check = format::unix_now();
                self.mpcorb_newer = false;
            }
            Ok(Freshness::Newer(_)) => self.mpcorb_newer = true,
            Err(_) => {}
        }
        if manual {
            self.mpcorb_fresh = Some(result);
        }
    }

    /// Once per session: the app update check (at most daily), and, with
    /// the offline source, a catalogue check when it is over a day old.
    /// Never retried in a loop, never downloads by itself.
    fn auto_checks(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() {
            return;
        }
        let now = format::unix_now();
        if !self.updates.auto_done && self.prefs.update_check {
            self.updates.auto_done = true;
            if now.saturating_sub(self.prefs.update_last_check) >= DAY {
                self.start_update_check(ctx, false);
            }
        }
        if !self.mpcorb_auto_done && self.prefs.known_source == KnownSource::Mpcorb {
            self.mpcorb_auto_done = true;
            if now.saturating_sub(self.prefs.mpcorb_last_check) >= DAY
                && known::catalogue_age().is_some_and(|age| age >= DAY)
            {
                self.start_mpcorb_check(ctx, false);
            }
        }
    }

    fn start_update_check(&mut self, ctx: &egui::Context, manual: bool) {
        if self.updates.pending.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::check());
            ctx.request_repaint();
        });
        self.updates.pending = Some(rx);
        self.updates.manual = manual;
        if manual {
            self.updates.error = None;
        }
    }

    fn poll_updates(&mut self) {
        let u = &mut self.updates;
        if let Some(rx) = &u.pending {
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("The update check stopped.".into())),
            };
            if let Some(result) = result {
                u.pending = None;
                match result {
                    Ok(update::Check::UpToDate) => {
                        let now = format::unix_now();
                        u.found = None;
                        u.error = None;
                        u.checked_at = Some(now);
                        // Only "nothing new" is remembered: an available
                        // update is checked (and shown) again next start.
                        self.prefs.update_last_check = now;
                    }
                    Ok(update::Check::Available(release)) => {
                        u.found = Some(release);
                        u.error = None;
                        u.checked_at = Some(format::unix_now());
                    }
                    Err(e) if u.manual => u.error = Some(e),
                    Err(_) => {} // silent at startup
                }
            }
        }
        if let Some(rx) = &u.downloading {
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err("The download stopped.".into())),
            };
            if let Some(result) = result {
                u.downloading = None;
                match result {
                    Ok(path) => {
                        update::reveal(&path);
                        u.downloaded = Some(path);
                    }
                    Err(e) => u.download_error = Some(e),
                }
            }
        }
    }

    fn start_update_download(&mut self, ctx: &egui::Context) {
        let u = &mut self.updates;
        let Some(asset) = u.found.as_ref().and_then(|r| r.asset.clone()) else {
            return;
        };
        if u.downloading.is_some() {
            return;
        }
        let Some(dir) = update::downloads_dir() else {
            u.download_error = Some("Couldn't find the Downloads folder.".into());
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::download(&asset, &dir));
            ctx.request_repaint();
        });
        u.downloading = Some(rx);
        u.downloaded = None;
        u.download_error = None;
    }

    fn open_settings(&mut self, page: Category) {
        self.settings_open = true;
        self.settings_page = page;
        self.settings_search.clear();
        self.help_open = false;
    }

    /// Applies changed defaults to the current session right away.
    fn apply_defaults(&mut self, before: Prefs, now: f64) {
        let p = self.prefs;
        if p.blink_seconds != before.blink_seconds {
            self.set_speed(p.blink_seconds, now);
        }
        if p.show_stars != before.show_stars {
            self.show_stars = p.show_stars;
        }
        if p.invert != before.invert {
            self.invert = p.invert;
        }
        if p.grid_size != before.grid_size {
            self.set_grid_size(p.grid_size);
        }
    }

    fn start_download(&mut self, ctx: &egui::Context) {
        if self.download.is_none() {
            self.download = Some(known::start_download(ctx));
            self.download_error = None;
            self.download_note = None;
        }
    }

    /// Centres the view on known object `i` (on the current frame, else
    /// wherever it is in the image), zooming in to at least 100 %.
    fn focus_known(&mut self, i: usize) {
        let Some(KnownState::Done(k)) = &self.known else {
            return;
        };
        let Some(o) = k.objects.get(i) else { return };
        let Some((x, y)) =
            o.at.get(self.index)
                .copied()
                .flatten()
                .or_else(|| o.at.iter().find_map(|p| *p))
        else {
            return;
        };
        let Some(h) = self.current().map(|f| f.height as f32) else {
            return;
        };
        self.known_focus = Some(i);
        let v = &mut self.view;
        v.auto_fit = false;
        v.scale = v.scale.max(1.0);
        v.offset = v.stage / 2.0 - vec2(x as f32 + 0.5, h - 0.5 - y as f32) * v.scale;
    }

    fn fit(&mut self) {
        let Some(f) = self.current() else { return };
        let (w, h) = (f.width as f32, f.height as f32);
        let stage = self.view.stage;
        let scale = fit_scale(stage, w, h);
        self.view.scale = scale;
        self.view.offset = (stage - vec2(w, h) * scale) / 2.0;
        self.view.auto_fit = true;
    }

    /// The stage changed size (side panel opened or closed, window
    /// resized): fit again if the view is still as fitted, else keep the
    /// image point at the centre where it is.
    fn resize_stage(&mut self, size: Vec2) {
        let old = std::mem::replace(&mut self.view.stage, size);
        if !self.view.fitted || old.x <= 0.0 || old.y <= 0.0 {
            return;
        }
        if self.view.auto_fit {
            self.fit();
        } else {
            self.view.offset += (size - old) / 2.0;
        }
    }

    /// Zooms by `factor` keeping the stage point `at` fixed (default: the centre).
    fn zoom_by(&mut self, factor: f32, at: Option<Vec2>) {
        let Some(f) = self.current() else { return };
        let min = fit_scale(self.view.stage, f.width as f32, f.height as f32) / 4.0;
        let v = &mut self.view;
        v.auto_fit = false;
        let at = at.unwrap_or(v.stage / 2.0);
        let next = (v.scale * factor).clamp(min, MAX_SCALE);
        v.offset = at - (at - v.offset) * (next / v.scale);
        v.scale = next;
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.theme != Some(self.prefs.theme) {
            ctx.set_theme(match self.prefs.theme {
                ThemeChoice::System => ThemePreference::System,
                ThemeChoice::Light => ThemePreference::Light,
                ThemeChoice::Dark => ThemePreference::Dark,
            });
            self.theme = Some(self.prefs.theme);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if ctx.text_edit_focused() {
            return;
        }
        let (events, now, command_held) = ctx.input(|i| {
            let m = i.modifiers;
            (i.events.clone(), i.time, m.command || m.ctrl || m.mac_cmd)
        });
        let mut handled = false;
        for event in events {
            match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let command = modifiers.command || modifiers.ctrl || modifiers.mac_cmd;
                    if command && !modifiers.alt {
                        match key {
                            Key::O if !self.settings_open => self.choose(ctx, frame),
                            Key::Comma => {
                                self.settings_open = !self.settings_open;
                                self.help_open = false;
                            }
                            _ => continue,
                        }
                        handled = true;
                        continue;
                    }
                    if key == Key::Escape && (self.help_open || self.settings_open) {
                        self.help_open = false;
                        self.settings_open = false;
                        handled = true;
                    }
                    if self.settings_open || modifiers.alt || command {
                        continue;
                    }
                    handled |= match key {
                        Key::Space => {
                            self.toggle_play(now);
                            true
                        }
                        Key::ArrowLeft => {
                            self.step(-1);
                            true
                        }
                        Key::ArrowRight => {
                            self.step(1);
                            true
                        }
                        Key::Tab if !modifiers.shift => {
                            self.prefs.panel_open = !self.prefs.panel_open;
                            true
                        }
                        _ => false,
                    };
                }
                // Characters, not keys, so `,` `.` `+` `?` work on any layout.
                Event::Text(text) if !self.settings_open && !command_held => {
                    handled = true;
                    match text.as_str() {
                        "+" | "=" => self.zoom_by(1.25, None),
                        "-" | "_" => self.zoom_by(0.8, None),
                        "0" => self.fit(),
                        "s" | "S" => self.show_stars = !self.show_stars,
                        "i" | "I" => self.invert = !self.invert,
                        "g" | "G" => self.set_grid(!self.grid_on),
                        "k" | "K" => self.set_known(!self.known_on),
                        "," | "[" => self.change_speed(1, now),
                        "." | "]" => self.change_speed(-1, now),
                        "o" | "O" => self.choose(ctx, frame),
                        "?" => self.help_open = !self.help_open,
                        "\\" => self.prefs.panel_open = !self.prefs.panel_open,
                        t => match t.parse::<usize>() {
                            Ok(n @ 1..=9) if n <= self.frame_count() => {
                                self.playing = false;
                                self.index = n - 1;
                            }
                            _ => handled = false,
                        },
                    }
                }
                _ => {}
            }
        }
        if handled {
            // Keep Space/arrows/Tab from also acting on whatever has focus.
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }

    fn tick_blink(&mut self, ctx: &egui::Context) {
        let n = self.frame_count();
        if !self.playing || n < 2 || self.loading.is_some() {
            return;
        }
        let now = ctx.input(|i| i.time);
        // egui wakes us up to one frame early (it subtracts the predicted
        // frame time from the delay); switching then instead of asking for
        // the last few milliseconds avoids a burst of extra repaints.
        if now + SWITCH_EARLY >= self.next_switch {
            self.index = (self.index + 1) % n;
            self.next_switch += self.period();
            if self.next_switch <= now {
                self.next_switch = now + self.period();
            }
        }
        ctx.request_repaint_after(Duration::from_secs_f64(self.next_switch - now));
    }

    /// Developer screenshots (see [`crate::capture`]): sets the requested UI
    /// state once the image set has loaded, then saves the window and quits.
    fn drive_capture(&mut self, ctx: &egui::Context) {
        let Some(cap) = &mut self.capture else { return };
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            match capture::write_png(&cap.path, &image) {
                Ok(()) => eprintln!("screenshot saved to {}", cap.path),
                Err(e) => eprintln!("screenshot: {}: {e}", cap.path),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        if self.loading.is_some() || matches!(self.known, Some(KnownState::Pending(_))) {
            return;
        }
        cap.frames += 1;
        let now = ctx.input(|i| i.time);
        if cap.frames > CAPTURE_SETTLE
            && let Some(label) = cap.focus.take()
        {
            cap.applied_at = Some(now);
            if let Some(KnownState::Done(k)) = &self.known
                && let Some(i) = k.objects.iter().position(|o| o.object.label() == label)
            {
                self.focus_known(i);
            }
            return;
        }
        let settled = cap.applied_at.is_some_and(|t| now - t >= CAPTURE_WAIT);
        if cap.frames >= CAPTURE_SHOOT && settled && !cap.requested {
            cap.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if cap.frames != CAPTURE_SETTLE {
            return;
        }
        cap.applied_at = Some(now);
        let s = std::mem::take(&mut cap.state);
        cap.focus = s.focus;
        if s.known {
            self.set_known(true);
        }
        self.show_stars |= s.stars;
        self.invert |= s.invert;
        if let Some(n) = s.grid_size.filter(|n| GRID_SIZES.contains(n)) {
            self.set_grid_size(n);
        }
        if s.grid {
            self.set_grid(true);
        }
        self.prefs.panel_open |= s.panel;
        self.marks = s.marks;
        if s.paused || s.frame.is_some() {
            self.playing = false;
        }
        if let Some(n) = s.frame.filter(|&n| n >= 1 && n <= self.frame_count()) {
            self.index = n - 1;
        }
        self.settings_open = s.settings;
        if let Some(page) = s.settings_page.as_deref().and_then(Category::from_key) {
            self.settings_page = page;
        }
        self.settings_search = s.search.unwrap_or_default();
        if let Some(version) = s.update {
            // A made-up release, to see how an available update looks.
            self.updates.found = Some(update::Release {
                page: String::new(),
                asset: update::platform_suffix().map(|suffix| update::Asset {
                    name: format!("OpenCeres-{version}{suffix}"),
                    url: String::new(),
                    size: 7_900_000,
                    sha256: None,
                }),
                version,
            });
        }
        self.help_open = s.help;
    }

    // ----------------------------------------------------------------------
    // Layout

    fn toolbar(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = palette(ui);
        egui::Panel::top("toolbar")
            .exact_size(theme::TOOLBAR_HEIGHT)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(S2 as i8, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let open = ui
                        .add_enabled(self.loading.is_none(), Button::new("Open…"))
                        .on_hover_text(format!("Open an image set ({})", command_key("O")));
                    if open.clicked() {
                        self.choose(ui.ctx(), frame);
                    }
                    ui.add_space(S3);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Right to left: the last item added is the leftmost.
                        let gear = icons::button(ui, Icon::Gear, self.settings_open, true)
                            .on_hover_text(format!("Settings ({})", command_key(",")));
                        if gear.clicked() {
                            self.settings_open = !self.settings_open;
                        }
                        let panel = icons::button(ui, Icon::Sidebar, self.prefs.panel_open, true)
                            .on_hover_text("Report panel (Tab)");
                        if panel.clicked() {
                            self.prefs.panel_open = !self.prefs.panel_open;
                        }
                        if let Some(r) = &self.updates.found {
                            ui.add_space(S1);
                            let pill = update_pill(ui, &format!("Update {}", r.version))
                                .on_hover_text("A new version of OpenCeres is available");
                            if pill.clicked() {
                                self.open_settings(Category::Updates);
                            }
                        }
                        if self.set.is_some() {
                            ui.add_space(S4);
                            self.view_toggles(ui);
                            ui.add_space(S4);
                            self.blink_controls(ui);
                        }
                        ui.add_space(S3);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            self.set_info(ui);
                        });
                    });
                });
            });
    }

    /// Set name, then its summary in muted text, cut with … when narrow.
    fn set_info(&self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let Some(set) = &self.set else {
            if let Some(l) = &self.loading {
                ui.label(RichText::new(&l.message).color(p.muted));
            }
            return;
        };
        let name = if set.name.is_empty() {
            "Image set"
        } else {
            set.name.as_str()
        };
        let mut job = LayoutJob::default();
        let format = |size: f32, color: Color32| TextFormat {
            font_id: FontId::proportional(size),
            color,
            valign: Align::Center,
            ..Default::default()
        };
        job.append(name, 0.0, format(13.5, p.text));
        job.append(&set.summary, S3, format(12.5, p.muted));
        ui.add(Label::new(job).truncate())
            .on_hover_text(format!("{name} · {}", set.summary));
    }

    /// `‹ ⏯ ›  2 / 4   − 0.5 s +`, added right to left.
    fn blink_controls(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let now = ui.input(|i| i.time);
        let speed_tip = "Blink speed, seconds per frame ( , slower · . faster )";
        let secs = self.blink_seconds;
        if icons::button(ui, Icon::Plus, false, secs > SPEEDS[0])
            .on_hover_text(speed_tip)
            .clicked()
        {
            self.change_speed(-1, now);
        }
        ui.add_sized(
            vec2(40.0, theme::TOOL),
            Label::new(RichText::new(format!("{secs} s")).color(p.text)),
        )
        .on_hover_text(speed_tip);
        if icons::button(ui, Icon::Minus, false, secs < SPEEDS[SPEEDS.len() - 1])
            .on_hover_text(speed_tip)
            .clicked()
        {
            self.change_speed(1, now);
        }
        ui.add_space(S3);
        let n = self.frame_count();
        let time = self
            .current()
            .map(|f| format::obs_time(f.date_obs.as_deref(), f.mjd_obs))
            .unwrap_or_default();
        ui.add_sized(
            vec2(40.0, theme::TOOL),
            Label::new(RichText::new(format!("{} / {n}", self.index + 1)).color(p.text)),
        )
        .on_hover_text(format!(
            "Frame {} of {n}, {time} (keys 1–{n})",
            self.index + 1
        ));
        if icons::button(ui, Icon::Next, false, n > 1)
            .on_hover_text("Next frame (→)")
            .clicked()
        {
            self.step(1);
        }
        let play = if self.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        let tip = if self.playing {
            "Pause (Space)"
        } else {
            "Blink (Space)"
        };
        if icons::button(ui, play, false, n > 1)
            .on_hover_text(tip)
            .clicked()
        {
            self.toggle_play(now);
        }
        if icons::button(ui, Icon::Prev, false, n > 1)
            .on_hover_text("Previous frame (←)")
            .clicked()
        {
            self.step(-1);
        }
    }

    /// Stars, Invert, Grid, Known toggles, added right to left.
    fn view_toggles(&mut self, ui: &mut egui::Ui) {
        if icons::button(ui, Icon::Known, self.known_on, true)
            .on_hover_text("Known asteroids and comets in the field (K)")
            .clicked()
        {
            self.set_known(!self.known_on);
        }
        if icons::button(ui, Icon::Grid, self.grid_on, true)
            .on_hover_text("Report grid (G)")
            .clicked()
        {
            self.set_grid(!self.grid_on);
        }
        if icons::button(ui, Icon::Invert, self.invert, true)
            .on_hover_text("Invert: dark stars on a white sky (I)")
            .clicked()
        {
            self.invert = !self.invert;
        }
        if icons::button(ui, Icon::Stars, self.show_stars, true)
            .on_hover_text("Detected stars (S)")
            .clicked()
        {
            self.show_stars = !self.show_stars;
        }
    }

    fn error_banner(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.error.clone() else {
            return;
        };
        let p = palette(ui);
        egui::Panel::top("error")
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.danger_bg)
                    .inner_margin(Margin::symmetric(S3 as i8, S1 as i8)),
            )
            .show(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icons::small_button(ui, Icon::Close, p.danger)
                        .on_hover_text("Dismiss")
                        .clicked()
                    {
                        self.error = None;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(RichText::new(error).color(p.danger)).wrap());
                    });
                });
            });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        let p = palette(ui);
        egui::Panel::bottom("status")
            .exact_size(theme::STATUS_HEIGHT)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(S3 as i8, 0)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = S4;
                    let small = |t: String| RichText::new(t).small().color(p.muted);
                    let current = self.current();
                    if let Some((x, y)) = self.hover {
                        ui.label(small(format!("x {}  y {}", x.round(), y.round())));
                        let sky = current.and_then(|f| f.wcs.as_ref()).map_or_else(
                            || "No sky coordinates in this image".to_owned(),
                            |wcs| {
                                let (ra, dec) = wcs.pixel_to_sky(x, y);
                                format!("RA {}   Dec {}", format::ra(ra), format::dec(dec))
                            },
                        );
                        ui.label(small(sky));
                    } else if self.set.is_some() {
                        ui.label(small("Point at the image for coordinates".into()));
                    }
                    if let Some(f) = current {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let zoom = (self.view.scale * 100.0).round();
                            ui.label(small(format!("{zoom}%")));
                            ui.label(small(format!("{} stars", thousands(f.stars.len()))));
                            if matches!(self.known, Some(KnownState::Pending(_))) {
                                ui.label(small("Looking up known objects…".into()));
                                ui.add(egui::Spinner::new().size(11.0).color(p.muted));
                            }
                        });
                    }
                });
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        if !self.prefs.panel_open {
            return;
        }
        let p = palette(ui);
        egui::Panel::right("panel")
            .exact_size(theme::PANEL_WIDTH)
            .resizable(false)
            .frame(Frame::new().fill(p.panel).inner_margin(Margin {
                left: S4 as i8,
                right: S4 as i8,
                top: S3 as i8,
                bottom: S3 as i8,
            }))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = S2;
                let count = (!self.marks.is_empty()).then_some(self.marks.len());
                section_heading(ui, "Easy report", count);
                self.report_section(ui);
                if self.known_on {
                    ui.add_space(S3);
                    hline(ui, p.border);
                    ui.add_space(S2);
                    let count = match &self.known {
                        Some(KnownState::Done(k)) => Some(k.objects.len()),
                        _ => None,
                    };
                    section_heading(ui, "Known objects", count);
                    self.known_section(ui);
                }
                ui.add_space(S3);
                hline(ui, p.border);
                ui.add_space(S2);
                section_heading(ui, "Candidates", None);
                ui.add(
                    Label::new(
                        RichText::new(
                            "Automatic detection is coming. For now, blink and look for \
                             anything that moves.",
                        )
                        .small()
                        .color(p.muted),
                    )
                    .wrap(),
                );
            });
    }

    fn report_section(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let loaded = self.set.is_some();
        if !self.grid_on {
            muted_wrapped(
                ui,
                "Turn on the grid, then click each box where something moves. Boxes are \
                 named row × column, counted from the top-left.",
            );
            let show = ui
                .add_enabled(loaded, Button::new("Show grid"))
                .on_hover_text("Report grid (G)");
            if show.clicked() {
                self.set_grid(true);
            }
            return;
        }

        let mut size = self.grid_size;
        let options: Vec<(usize, String)> = GRID_SIZES
            .iter()
            .map(|&n| (n, format!("{n}×{n}")))
            .collect();
        if segmented(ui, &mut size, &options, ui.available_width()) {
            self.set_grid_size(size);
        }

        if self.marks.is_empty() {
            let hint = if loaded {
                "Click the boxes on the image where something moves."
            } else {
                "Open an image set first."
            };
            muted_wrapped(ui, hint);
        } else {
            Frame::new()
                .fill(p.inset)
                .corner_radius(theme::RADIUS)
                .inner_margin(Margin::symmetric(S2 as i8 + 2, S2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        Label::new(RichText::new(self.report()).monospace().color(p.text)).wrap(),
                    );
                });
        }

        let has_marks = !self.marks.is_empty();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            let w = (ui.available_width() - S2) / 2.0;
            let label = if self.copied { "Copied" } else { "Copy report" };
            let copy = ui
                .add_enabled(has_marks, Button::new(label).min_size(vec2(w, theme::TOOL)))
                .on_hover_text("Copy the report line to paste into the IASC form");
            if self.copied {
                let r = Rect::from_center_size(
                    copy.rect.left_center() + vec2(S4 + 2.0, 0.0),
                    vec2(14.0, 14.0),
                );
                icons::paint(ui.painter(), r, Icon::Check, p.accent);
            }
            if copy.clicked() {
                ui.ctx().copy_text(self.report());
                self.copied = true;
            }
            let clear = ui.add_enabled(
                has_marks,
                Button::new("Clear").min_size(vec2(w, theme::TOOL)),
            );
            if clear.clicked() {
                self.marks.clear();
                self.copied = false;
            }
        });
    }

    fn stage(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        if rect.size() != self.view.stage {
            self.resize_stage(rect.size());
        }

        if self.set.is_some() {
            ui.painter().rect_filled(rect, 0.0, theme::STAGE);
            self.viewer(ui, rect, &response);
            self.badge(ui, rect);
        } else {
            ui.painter().rect_filled(rect, 0.0, palette(ui).bg);
            self.hover = None;
            self.empty_state(ui, rect, frame);
        }
        self.overlays(ui, rect);
        if self.help_open {
            self.help_overlay(ui, rect);
        }
    }

    fn viewer(&mut self, ui: &mut egui::Ui, rect: Rect, response: &Response) {
        let Some((w, h)) = self.current().map(|f| (f.width as f32, f.height as f32)) else {
            return;
        };
        if !self.view.fitted && rect.width() > 0.0 && rect.height() > 0.0 {
            self.fit();
            self.view.fitted = true;
        }

        // Pointer: drag to pan, click (no drag) to mark a grid box, wheel to zoom.
        let (pos, pressed, down, released, events) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.events.clone(),
            )
        });
        let hovered = response.hovered();
        if pressed
            && hovered
            && let Some(start) = pos
        {
            self.view.drag = Some(Drag {
                start,
                offset: self.view.offset,
                moved: false,
            });
        }
        let mut pick = None;
        if let Some(drag) = &mut self.view.drag {
            if let Some(pos) = pos {
                let delta = pos - drag.start;
                drag.moved |= delta.length() > CLICK_SLOP;
                if drag.moved {
                    self.view.offset = drag.offset + delta;
                    self.view.auto_fit = false;
                }
                if released && !drag.moved {
                    pick = Some(pos);
                }
            }
            if released || !down {
                self.view.drag = None;
            }
        }
        if let Some(pos) = pick {
            let uv = (pos - rect.min - self.view.offset) / self.view.scale;
            if uv.x >= 0.0 && uv.y >= 0.0 && uv.x < w && uv.y < h {
                self.toggle_mark(uv.x / w, uv.y / h);
            }
        }
        if hovered {
            let mut factor = 1.0f32;
            for e in &events {
                match e {
                    Event::MouseWheel { unit, delta, .. } => {
                        let px = match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * 16.0,
                            egui::MouseWheelUnit::Page => delta.y * 800.0,
                        };
                        factor *= (px * 0.0015).exp();
                    }
                    Event::Zoom(z) => factor *= z,
                    _ => {}
                }
            }
            if factor != 1.0
                && let Some(pos) = pos
            {
                self.zoom_by(factor, Some(pos - rect.min));
            }
        }
        if self.view.drag.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        } else if hovered {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }

        self.hover = pos
            .filter(|p| (hovered || self.view.drag.is_some()) && rect.contains(*p))
            .map(|p| (p - rect.min - self.view.offset) / self.view.scale)
            .filter(|uv| uv.x >= 0.0 && uv.y >= 0.0 && uv.x < w && uv.y < h)
            .map(|uv| (f64::from(uv.x) - 0.5, f64::from(h) - f64::from(uv.y) - 0.5));

        // Paint.
        let scale = self.view.scale;
        let origin = rect.min + self.view.offset;
        let image = Rect::from_min_size(origin, vec2(w, h) * scale);
        let painter = ui.painter_at(rect);
        if image.intersects(rect) {
            painter.add(Shape::Callback(self.gpu.paint(
                ui.ctx(),
                rect,
                Draw {
                    index: self.index,
                    image,
                    size: [w as usize, h as usize],
                    invert: self.invert,
                },
            )));
        }

        if self.show_stars
            && let Some(f) = self.current()
        {
            let visible = rect.expand(12.0);
            let stroke = Stroke::new(1.25, theme::STAR);
            for s in &f.stars {
                let c = origin + vec2(s.x + 0.5, h - 0.5 - s.y) * scale;
                if visible.contains(c) {
                    painter.circle_stroke(c, (s.fwhm * 1.5 * scale).max(4.0), stroke);
                }
            }
        }

        if self.grid_on {
            self.paint_grid(&painter, image);
        }
        if self.known_on {
            self.paint_known(&painter, rect, origin, h);
        }
    }

    /// A ring (at least 6″ across the radius) and a `133105 · 19.0` chip for
    /// each known object on the current frame. Labels that would overlap an
    /// earlier (brighter) one are left out, and all of them when zoomed far out.
    fn paint_known(&self, painter: &egui::Painter, rect: Rect, origin: Pos2, h: f32) {
        let (Some(KnownState::Done(k)), Some(f)) = (&self.known, self.current()) else {
            return;
        };
        let scale = self.view.scale;
        let arcsec = f.wcs.as_ref().map_or(0.25, |w| w.pixel_scale()) as f32;
        let r = (6.0 / arcsec * scale).max(10.0);
        let font = FontId::proportional(11.5);
        let labels = scale >= 0.1;
        let mut taken: Vec<Rect> = Vec::new();
        let visible = rect.expand(r + 4.0);
        // The focused object last, so its ring and label are on top.
        let order = (0..k.objects.len())
            .filter(|&i| Some(i) != self.known_focus)
            .chain(self.known_focus);
        let mut chips = Vec::new();
        for i in order {
            let Some(o) = k.objects.get(i) else { continue };
            let Some((x, y)) = o.at.get(self.index).copied().flatten() else {
                continue;
            };
            let c = origin + vec2(x as f32 + 0.5, h - 0.5 - y as f32) * scale;
            if !visible.contains(c) {
                continue;
            }
            let focused = Some(i) == self.known_focus;
            let width = if focused { 2.25 } else { 1.5 };
            painter.circle_stroke(c, r, Stroke::new(width + 2.0, theme::KNOWN_HALO));
            painter.circle_stroke(c, r, Stroke::new(width, theme::KNOWN));
            if labels {
                chips.push((c, known_chip_text(&o.object), focused));
            }
        }
        // Labels after all rings, focused one first so it always shows.
        chips.sort_by_key(|&(_, _, focused)| !focused);
        for (c, text, _) in chips {
            let galley = painter.layout_no_wrap(text, font.clone(), theme::KNOWN_TEXT);
            // Up and right of the ring, flipped where that leaves the stage.
            let d = r * std::f32::consts::FRAC_1_SQRT_2;
            let size = vec2(galley.size().x + S2, 17.0);
            let mut min = c + vec2(d + 2.0, -d - 15.0);
            if min.x + size.x > rect.max.x {
                min.x = c.x - d - 2.0 - size.x;
            }
            if min.y < rect.min.y {
                min.y = c.y + d - 2.0;
            }
            let chip = Rect::from_min_size(min, size);
            if taken.iter().any(|t| t.intersects(chip)) {
                continue;
            }
            taken.push(chip);
            painter.rect_filled(chip, 3.0, theme::GRID_CHIP);
            let at = pos2(chip.min.x + S1, chip.center().y - galley.size().y / 2.0);
            painter.galley(at, galley, theme::KNOWN_TEXT);
        }
    }

    fn known_section(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        if self.set.is_none() {
            muted_wrapped(ui, "Open an image set first.");
            return;
        }
        match &self.known {
            None | Some(KnownState::Pending(_)) => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(13.0).color(p.muted));
                    ui.label(RichText::new("Looking up known objects…").color(p.muted));
                });
            }
            Some(KnownState::Failed(e)) => {
                ui.add(Label::new(RichText::new(e).color(p.danger)).wrap());
                if ui.button("Try again").clicked() {
                    self.known = None;
                }
            }
            Some(KnownState::NoCatalogue) => {
                muted_wrapped(
                    ui,
                    "The offline MPCORB catalogue isn't on this computer yet. Download it \
                     once (about 94 MB), or use SkyBoT online.",
                );
                self.download_controls(ui);
                if self.download.is_none()
                    && ui
                        .button("Use SkyBoT instead")
                        .on_hover_text("Changes the source in Settings")
                        .clicked()
                {
                    self.prefs.known_source = KnownSource::SkyBot;
                }
            }
            Some(KnownState::Done(k)) => {
                let mut clicked = None;
                if k.objects.is_empty() {
                    muted_wrapped(ui, "No known asteroids or comets in this field.");
                } else {
                    let max = (ui.available_height() - 120.0).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("known_list")
                        .max_height(max)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            for (i, o) in k.objects.iter().enumerate() {
                                let here = o.at.get(self.index).copied().flatten();
                                let detail = self.known_detail(&o.object, here);
                                let selected = self.known_focus == Some(i);
                                if known_row(ui, &o.object, &detail, selected).clicked() {
                                    clicked = Some(i);
                                }
                            }
                        });
                }
                let source = match (&k.source, &k.catalogue) {
                    (KnownSource::Mpcorb, Some(date)) => format!("MPCORB of {date}"),
                    (s, _) => s.short().to_owned(),
                };
                ui.add_space(S1);
                ui.label(
                    RichText::new(format!("{source} · looked up {}", k.at))
                        .small()
                        .color(p.muted),
                );
                let mut open = false;
                if k.source == KnownSource::Mpcorb && self.mpcorb_newer {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = S1;
                        ui.label(
                            RichText::new("Newer catalogue available")
                                .small()
                                .color(p.accent),
                        );
                        open = ui
                            .link(RichText::new("Update…").small().color(p.accent))
                            .on_hover_text("Settings → Known objects")
                            .clicked();
                    });
                }
                if open {
                    self.open_settings(Category::Known);
                }
                if let Some(i) = clicked {
                    self.focus_known(i);
                }
            }
        }
    }

    /// `MB>Inner · 0.64″/min · box 3x2` for a list row.
    fn known_detail(&self, o: &KnownObject, here: Option<(f64, f64)>) -> String {
        let rate = o.motion_ra.hypot(o.motion_dec) / 60.0;
        let mut parts = vec![o.class.clone(), format!("{rate:.2}″/min")];
        match (here, self.current()) {
            (Some((x, y)), Some(f)) if self.grid_on => {
                let (w, h) = (f.width as f32, f.height as f32);
                let u = (x as f32 + 0.5) / w;
                let v = (h - 0.5 - y as f32) / h;
                parts.push(format!("box {}", box_label(grid_box(u, v, self.grid_size))));
            }
            (None, _) => parts.push("not on this frame".into()),
            _ => {}
        }
        parts.join(" · ")
    }

    /// Download button, or progress with a Cancel button while downloading.
    fn download_controls(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        if let Some(d) = &self.download {
            use std::sync::atomic::Ordering::Relaxed;
            let mb = |b: u64| b as f64 / 1e6;
            let (got, total) = (d.received.load(Relaxed), d.total.load(Relaxed));
            let mut cancel = false;
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 72.0).max(80.0);
                if d.checking.load(Relaxed) {
                    ui.add(egui::Spinner::new().size(13.0).color(p.muted));
                    ui.label(RichText::new("Checking the download…").color(p.muted));
                } else if total > 0 {
                    let frac = (got as f32 / total as f32).clamp(0.0, 1.0);
                    ui.add(egui::ProgressBar::new(frac).desired_width(w).text(format!(
                        "{:.0} of {:.0} MB",
                        mb(got),
                        mb(total)
                    )));
                } else {
                    ui.add(egui::Spinner::new().size(13.0).color(p.muted));
                    ui.label(RichText::new(format!("{:.0} MB", mb(got))).color(p.muted));
                }
                cancel = ui.button("Cancel").clicked();
            });
            if cancel {
                d.cancel();
            }
        } else {
            let label = if matches!(self.catalogue, Some(Catalogue::Present { .. })) {
                "Update MPCORB"
            } else {
                "Download MPCORB"
            };
            if ui
                .button(label)
                .on_hover_text(
                    "Download the latest catalogue from the Minor Planet Center (~94 MB)",
                )
                .clicked()
            {
                self.start_download(ui.ctx());
            }
        }
        if let Some(e) = &self.download_error {
            ui.add(Label::new(RichText::new(e).small().color(p.danger)).wrap());
        } else if let Some(note) = &self.download_note {
            ui.add(Label::new(RichText::new(note).small().color(p.muted)).wrap());
        }
    }

    fn toggle_mark(&mut self, u: f32, v: f32) {
        if !self.grid_on {
            return;
        }
        let key = grid_box(u, v, self.grid_size);
        if let Some(i) = self.marks.iter().position(|&m| m == key) {
            self.marks.remove(i);
        } else {
            self.marks.push(key);
        }
        self.copied = false;
    }

    fn paint_grid(&self, painter: &egui::Painter, image: Rect) {
        let n = self.grid_size;
        let cell = image.size() / n as f32;
        for &(r, c) in &self.marks {
            let min = image.min + vec2((c - 1) as f32 * cell.x, (r - 1) as f32 * cell.y);
            painter.rect_filled(Rect::from_min_size(min, cell), 0.0, theme::GRID_FILL);
        }
        let stroke = Stroke::new(1.0, theme::GRID_LINE);
        for i in 0..=n {
            let x = image.min.x + i as f32 * cell.x;
            let y = image.min.y + i as f32 * cell.y;
            painter.vline(x, image.y_range(), stroke);
            painter.hline(image.x_range(), y, stroke);
        }
        if cell.min_elem() < 36.0 {
            return; // too small to label legibly
        }
        let font = FontId::proportional(11.5);
        for r in 1..=n {
            for c in 1..=n {
                let at = image.min
                    + vec2(
                        (c - 1) as f32 * cell.x + S1 + 1.0,
                        (r - 1) as f32 * cell.y + S1 + 1.0,
                    );
                let galley =
                    painter.layout_no_wrap(box_label((r, c)), font.clone(), theme::GRID_TEXT);
                let chip = Rect::from_min_size(at, vec2(galley.size().x + S2, 17.0));
                painter.rect_filled(chip, 3.0, theme::GRID_CHIP);
                let text_at = pos2(at.x + S1, chip.center().y - galley.size().y / 2.0);
                painter.galley(text_at, galley, theme::GRID_TEXT);
            }
        }
    }

    /// Small, low-contrast `2 / 4 · 10:33:17 UTC` in the bottom-left corner.
    fn badge(&self, ui: &egui::Ui, rect: Rect) {
        let Some(f) = self.current() else { return };
        let text = format!(
            "{} / {}  ·  {}",
            self.index + 1,
            self.frame_count(),
            format::clock(f.date_obs.as_deref(), f.mjd_obs)
        );
        let painter = ui.painter_at(rect);
        let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), theme::BADGE_TEXT);
        let size = galley.size() + vec2(2.0 * S2, 2.0 * S1);
        let chip = Rect::from_min_size(pos2(rect.min.x + S3, rect.max.y - S3 - size.y), size);
        painter.rect_filled(chip, theme::RADIUS, theme::BADGE);
        painter.galley(chip.min + vec2(S2, S1), galley, theme::BADGE_TEXT);
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, rect: Rect, frame: &eframe::Frame) {
        let p = palette(ui);
        let size = vec2(
            440.0f32.min(rect.width() - 2.0 * S4),
            216.0f32.min(rect.height() - 2.0 * S4),
        );
        if size.x < 200.0 || size.y < 120.0 {
            return;
        }
        let card = Rect::from_center_size(rect.center() - vec2(0.0, S5), size);
        let dropping = ui.input(|i| !i.raw.hovered_files.is_empty());
        let (fill, stroke) = if dropping {
            (p.accent_weak, Stroke::new(2.0, p.accent))
        } else {
            (p.panel, Stroke::new(1.0, p.border))
        };
        ui.painter()
            .rect(card, theme::RADIUS, fill, stroke, StrokeKind::Inside);

        let mut choose = false;
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(card.shrink2(vec2(S5, S5)))
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.y = S1;
                let (icon, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                paint_logo(ui, icon);
                ui.add_space(S3);
                let title = if dropping {
                    "Release to open the image set"
                } else {
                    "Drop your IASC image set here"
                };
                ui.label(RichText::new(title).size(17.0).color(p.text));
                ui.label(
                    RichText::new("The .zip from IASC, or 3–4 FITS images of the same field")
                        .color(p.muted),
                );
                ui.add_space(S4);
                let button = ui.add_enabled(
                    self.loading.is_none(),
                    Button::new(RichText::new("Choose files…").color(p.accent_text))
                        .fill(p.accent)
                        .stroke(Stroke::NONE)
                        .min_size(vec2(132.0, 30.0)),
                );
                choose = button.clicked();
                ui.add_space(S4);
                ui.label(
                    RichText::new("O open  ·  Space blink  ·  ← → step  ·  ? all shortcuts")
                        .small()
                        .color(p.muted),
                );
            },
        );
        if choose {
            self.choose(ui.ctx(), frame);
        }
    }

    /// Loading spinner, and the drop highlight over a loaded set.
    fn overlays(&self, ui: &mut egui::Ui, rect: Rect) {
        let dropping = ui.input(|i| !i.raw.hovered_files.is_empty()) && self.set.is_some();
        let message = match (&self.loading, dropping) {
            (_, true) => "Release to open the image set",
            (Some(l), false) => l.message.as_str(),
            (None, false) => return,
        };
        let accent = palette(ui).accent;
        egui::Area::new(Id::new("overlay"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                let (r, _) = ui.allocate_exact_size(rect.size(), Sense::click_and_drag());
                let painter = ui.painter();
                let font = FontId::proportional(15.0);
                if dropping {
                    let zone = r.shrink(S3);
                    painter.rect_filled(zone, theme::RADIUS, Color32::from_black_alpha(90));
                    painter.rect_stroke(
                        zone,
                        theme::RADIUS,
                        Stroke::new(2.0, accent),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        message,
                        font,
                        theme::OVERLAY_TEXT,
                    );
                } else {
                    painter.rect_filled(r, 0.0, Color32::from_black_alpha(120));
                    let galley =
                        painter.layout_no_wrap(message.to_owned(), font, theme::OVERLAY_TEXT);
                    let total = 18.0 + 10.0 + galley.size().x;
                    let left = r.center().x - total / 2.0;
                    let spinner =
                        Rect::from_center_size(pos2(left + 9.0, r.center().y), vec2(18.0, 18.0));
                    egui::Spinner::new()
                        .size(18.0)
                        .color(Color32::WHITE)
                        .paint_at(ui, spinner);
                    let at = pos2(left + 28.0, r.center().y - galley.size().y / 2.0);
                    ui.painter().galley(at, galley, theme::OVERLAY_TEXT);
                }
            });
    }

    /// The `?` card: a small list of keys, dismissed with `?`, Esc or ×.
    fn help_overlay(&mut self, ui: &egui::Ui, rect: Rect) {
        let p = palette(ui);
        let ctx = ui.ctx().clone();
        let style = ctx.global_style();
        egui::Area::new(Id::new("help"))
            .order(Order::Foreground)
            .pivot(Align2::RIGHT_TOP)
            .fixed_pos(rect.right_top() + vec2(-S3, S3))
            .show(&ctx, |ui| {
                Frame::popup(&style)
                    .inner_margin(Margin::same(S4 as i8))
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Keyboard shortcuts").strong().color(p.text));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if icons::small_button(ui, Icon::Close, p.muted)
                                    .on_hover_text("Close (?)")
                                    .clicked()
                                {
                                    self.help_open = false;
                                }
                            });
                        });
                        ui.add_space(S2);
                        shortcut_table(ui, "help_keys");
                    });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.apply_theme(&ctx);
        self.poll_loading(now);
        self.poll_download();
        self.poll_mpcorb_check();
        self.poll_updates();
        self.auto_checks(&ctx);
        if self.prefs.known_source != self.saved.known_source {
            self.known = None; // look up again from the new source
        }
        self.poll_known();
        self.ensure_known(&ctx);
        self.drive_capture(&ctx);
        if let Some(e) = self.gpu.take_error() {
            self.error = Some(e);
        }
        let dropped: Vec<String> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_string_lossy().into_owned())
                .collect()
        });
        if !dropped.is_empty() {
            self.open(&ctx, dropped);
        }
        self.handle_keys(&ctx, frame);
        self.tick_blink(&ctx);

        self.toolbar(ui, frame);
        self.error_banner(ui);
        self.status_bar(ui);
        self.side_panel(ui);
        egui::CentralPanel::no_frame().show(ui, |ui| self.stage(ui, frame));
        self.settings_window(&ctx, now);

        if self.prefs != self.saved {
            if self.capture.is_none() {
                self.prefs.save();
            }
            self.saved = self.prefs;
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(gl) = gl {
            self.gpu.destroy(gl);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::STAGE.to_normalized_gamma_f32()
    }
}

/// `4 frames · 2423×2434 · 0.26″/px · w · 52.7 min`
fn summary(frames: &[ImageFrame]) -> String {
    let Some(first) = frames.first() else {
        return String::new();
    };
    let n = frames.len();
    let mut parts = vec![
        format!("{n} frame{}", if n > 1 { "s" } else { "" }),
        format!("{}×{}", first.width, first.height),
    ];
    if let Some(wcs) = &first.wcs {
        parts.push(format!("{:.2}″/px", wcs.pixel_scale()));
    }
    if let Some(filter) = &first.filter {
        parts.push(filter.clone());
    }
    let times: Option<Vec<f64>> = frames.iter().map(|f| f.jd).collect();
    if let Some(times) = times.filter(|t| t.len() > 1) {
        let max = times.iter().copied().fold(f64::MIN, f64::max);
        let min = times.iter().copied().fold(f64::MAX, f64::min);
        parts.push(format!("{:.1} min", (max - min) * 1440.0));
    }
    parts.join("  ·  ")
}

/// Places the objects of a lookup on each frame (at that frame's
/// mid-exposure time) and keeps those inside the image, brightest first.
fn place(lookup: Lookup, frames: &[ImageFrame]) -> KnownField {
    let epoch = lookup.epoch;
    let mut objects: Vec<Placed> = lookup
        .objects
        .into_iter()
        .filter_map(|object| {
            let at: Vec<_> = frames
                .iter()
                .map(|f| {
                    let (ra, dec) = object.position_at(epoch, f.mid_jd.unwrap_or(epoch));
                    let (x, y) = f.wcs.as_ref()?.sky_to_pixel(ra, dec)?;
                    let (w, h) = (f.width as f64 - 0.5, f.height as f64 - 0.5);
                    (x >= -0.5 && y >= -0.5 && x < w && y < h).then_some((x, y))
                })
                .collect();
            at.iter()
                .any(Option::is_some)
                .then_some(Placed { object, at })
        })
        .collect();
    objects.sort_by(|a, b| {
        let m = |o: &Placed| o.object.magnitude.unwrap_or(f64::INFINITY);
        m(a).total_cmp(&m(b))
    });
    KnownField {
        objects,
        source: lookup.source,
        at: lookup.at,
        catalogue: lookup.catalogue,
    }
}

/// `133105 · 19.0`, the label drawn next to a known object.
fn known_chip_text(o: &KnownObject) -> String {
    match o.magnitude {
        Some(m) => format!("{} · {m:.1}", o.label()),
        None => o.label().to_owned(),
    }
}

/// One clickable list row: label (and name), magnitude, then details.
fn known_row(ui: &mut egui::Ui, o: &KnownObject, detail: &str, selected: bool) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, theme::RADIUS, p.accent_weak);
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, p.hover);
    }
    let inner = rect.shrink2(vec2(S2, S1 + 1.0));
    let one_line = |job: &mut LayoutJob, width: f32| {
        job.wrap = egui::text::TextWrapping {
            max_width: width,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
    };
    let format = |size: f32, color: Color32| TextFormat {
        font_id: FontId::proportional(size),
        color,
        ..Default::default()
    };
    let mag = o.magnitude.map(|m| format!("{m:.1}")).unwrap_or_default();
    let mag = painter.layout_no_wrap(mag, FontId::proportional(13.0), p.text);
    let mut title = LayoutJob::default();
    title.append(o.label(), 0.0, format(13.0, p.text));
    if o.number.is_some() {
        title.append(&o.name, S2, format(12.0, p.muted));
    }
    one_line(&mut title, inner.width() - mag.size().x - S2);
    let mut sub = LayoutJob::default();
    sub.append(detail, 0.0, format(11.5, p.muted));
    one_line(&mut sub, inner.width());
    let title = painter.layout_job(title);
    let sub = painter.layout_job(sub);
    painter.galley(pos2(inner.max.x - mag.size().x, inner.min.y), mag, p.text);
    painter.galley(inner.min, title, p.text);
    painter.galley(pos2(inner.min.x, inner.max.y - sub.size().y), sub, p.muted);
    let tip = format!(
        "{}{} — click to centre on it",
        o.name,
        o.number
            .as_deref()
            .map_or_else(String::new, |n| format!(" ({n})"))
    );
    response
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tip)
}

/// Report box (row, column), 1-based from the top-left, holding the point
/// at fractions `u` across and `v` down the displayed image.
fn grid_box(u: f32, v: f32, n: usize) -> (usize, usize) {
    let cell = |t: f32| ((t.max(0.0) * n as f32).floor() as usize + 1).min(n);
    (cell(v), cell(u))
}

/// The team's box name: row × column, e.g. `1x2` = top row, second column.
fn box_label((row, col): (usize, usize)) -> String {
    format!("{row}x{col}")
}

/// e.g. `XY57_p01: 1x2 2x1`, boxes in reading order (by row, then column).
fn report_line(name: &str, marks: &[(usize, usize)]) -> String {
    let mut boxes = marks.to_vec();
    boxes.sort_unstable();
    let boxes: Vec<String> = boxes.into_iter().map(box_label).collect();
    format!("{name}: {}", boxes.join(" "))
}

fn command_key(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{key}")
    } else {
        format!("Ctrl+{key}")
    }
}

fn fit_scale(stage: Vec2, w: f32, h: f32) -> f32 {
    let s = (stage.x / w).min(stage.y / h) * 0.98;
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
}

/// `12345` → `12,345`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A row of joined toggle buttons; returns true when the value changed.
fn segmented<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, String)],
    width: f32,
) -> bool {
    let p = palette(ui);
    let mut changed = false;
    let gap = 2.0;
    let n = options.len() as f32;
    let w = ((width - 2.0 * gap - gap * (n - 1.0)) / n).max(28.0);
    Frame::new()
        .fill(p.inset)
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::same(gap as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (option, label) in options {
                    let selected = *option == *value;
                    let text = RichText::new(label).size(12.0);
                    let (text, fill, stroke) = if selected {
                        (text.color(p.text), p.panel, Stroke::new(1.0, p.border))
                    } else {
                        (text.color(p.muted), Color32::TRANSPARENT, Stroke::NONE)
                    };
                    let b = Button::new(text)
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(theme::RADIUS - 2)
                        .min_size(vec2(w, theme::TOOL - 6.0));
                    if ui.add(b).clicked() && !selected {
                        *value = *option;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// The calm toolbar notice of an available update.
fn update_pill(ui: &mut egui::Ui, text: &str) -> Response {
    let p = palette(ui);
    let button = Button::new(RichText::new(text).size(12.0).color(p.accent))
        .fill(p.accent_weak)
        .stroke(Stroke::NONE)
        .corner_radius(11)
        .min_size(vec2(0.0, 22.0));
    ui.add(button).on_hover_cursor(CursorIcon::PointingHand)
}

fn section_heading(ui: &mut egui::Ui, title: &str, count: Option<usize>) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong().color(p.text));
        if let Some(n) = count {
            Frame::new()
                .fill(p.accent_weak)
                .corner_radius(8)
                .inner_margin(Margin::symmetric(6, 0))
                .show(ui, |ui| {
                    ui.label(RichText::new(n.to_string()).small().color(p.accent));
                });
        }
    });
}

fn muted_wrapped(ui: &mut egui::Ui, text: &str) {
    let p = palette(ui);
    ui.add(Label::new(RichText::new(text).color(p.muted)).wrap());
}

fn shortcut_table(ui: &mut egui::Ui, id: &str) {
    let p = palette(ui);
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(vec2(S3, S1 + 2.0))
        .min_col_width(64.0)
        .show(ui, |ui| {
            for (keys, action) in shortcuts() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = S1;
                    for k in &keys {
                        if k == "–" {
                            ui.label(RichText::new(k).small().color(p.muted));
                        } else {
                            keycap(ui, k);
                        }
                    }
                });
                ui.label(RichText::new(action).color(p.text));
                ui.end_row();
            }
        });
}

fn keycap(ui: &mut egui::Ui, key: &str) {
    let p = palette(ui);
    Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(4)
        .inner_margin(Margin::symmetric(5, 0))
        .show(ui, |ui| {
            ui.label(RichText::new(key).size(11.5).color(p.text));
        });
}

fn hline(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color);
}

/// The app's mark: a dark disc crossed by a bright streak.
fn paint_logo(ui: &egui::Ui, rect: Rect) {
    let p = palette(ui);
    let s = rect.width() / 32.0;
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * s;
    let painter = ui.painter();
    painter.circle_filled(at(16.0, 16.0), 13.0 * s, p.disc);
    painter.line_segment(
        [at(9.0, 23.0), at(21.0, 11.0)],
        Stroke::new(3.0 * s, theme::STREAK),
    );
    let cap = 1.5 * s;
    painter.circle_filled(at(9.0, 23.0), cap, theme::STREAK);
    painter.circle_filled(at(21.0, 11.0), cap, theme::STREAK);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_are_row_by_column() {
        // Top row, second column of a 4×4 grid.
        assert_eq!(box_label(grid_box(0.3, 0.1, 4)), "1x2");
        assert_eq!(box_label(grid_box(0.1, 0.3, 4)), "2x1");
        assert_eq!(box_label(grid_box(1.0, 1.0, 4)), "4x4");
        let marks = [grid_box(0.1, 0.3, 4), grid_box(0.3, 0.1, 4)];
        assert_eq!(report_line("XY57_p01", &marks), "XY57_p01: 1x2 2x1");
    }

    #[test]
    fn groups_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
