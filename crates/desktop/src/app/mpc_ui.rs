//! "Report for IASC": the MPC report of the open search. Gaia stars are
//! fetched and the set measured in the background ([`crate::gaia`],
//! [`crate::measure`]); the side panel shows one state line and opens the
//! review, where the student checks each object and copies the report to
//! paste into IASC's submission box.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Id, Label, Layout, Margin, RichText, ScrollArea,
    Stroke, TextEdit, vec2,
};
use openceres_core::gaia::{GaiaStar, MIRRORS};
use openceres_core::mpc;

use super::{App, muted_wrapped};
use crate::gaia::{self, Fetch, Origin, TestResult};
use crate::icons::{self, Icon};
use crate::measure::{self, Job, Measured};
use crate::report::{self, Built, Check, Inputs};
use crate::settings::Category;
use crate::theme::{self, S1, S2, S3, S4, S5, palette};
use crate::update;

/// Gaia stars for the open set.
#[derive(Default)]
pub(super) enum GaiaState {
    #[default]
    Idle,
    Fetching(Fetch),
    Ready {
        stars: Arc<Vec<GaiaStar>>,
        origin: Origin,
    },
    Failed(String),
}

/// A report state forced for screenshots (`mpc=…` in the capture state).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Forced {
    Fetching,
    Measuring,
    Offline,
    Failed,
    Preview,
    Missing,
    Ready,
}

impl Forced {
    pub fn parse(s: &str) -> Option<Forced> {
        Some(match s {
            "fetching" => Forced::Fetching,
            "measuring" => Forced::Measuring,
            "offline" => Forced::Offline,
            "failed" => Forced::Failed,
            "preview" => Forced::Preview,
            "missing" => Forced::Missing,
            "ready" => Forced::Ready,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(super) struct Mpc {
    pub gaia: GaiaState,
    pub job: Option<Job>,
    pub measured: Option<Measured>,
    pub error: Option<String>,
    pub review_open: bool,
    /// e.g. "Saved OC report.txt" after a save.
    pub note: Option<String>,
    pub forced: Option<Forced>,
}

/// Mirror tests from Settings (kept across sets).
#[derive(Default)]
pub(super) struct MirrorTests {
    pub rx: Option<Receiver<(usize, TestResult)>>,
    pub results: [Option<TestResult>; MIRRORS.len()],
}

/// What the side panel says about the report.
#[derive(Debug, Clone, PartialEq)]
enum State {
    NoSet,
    NoObjects,
    Marking,
    Fetching {
        mirror: &'static str,
        seconds: u64,
    },
    Measuring,
    /// No Gaia stars and none cached.
    Offline(String),
    /// A frame with marks did not solve.
    SolveFailed(String),
    /// Positions from the header WCS only.
    Preview,
    /// Header details missing.
    Missing(Vec<&'static str>),
    /// Nothing measurable to report yet.
    Empty(String),
    Ready {
        warnings: usize,
    },
}

/// How a state looks: icon shape (never colour alone) and colour.
enum Tone {
    /// Waiting for the student (no animation: idle CPU stays at zero).
    Idle,
    /// A job is running (spinner).
    Busy,
    Good,
    Warn,
    Blocked,
}

impl App {
    /// The search's measurer: the "Measured by" of the open search.
    fn measured_by(&self) -> String {
        self.searches
            .record
            .as_ref()
            .map_or_else(String::new, |r| r.searched_by.trim().to_owned())
    }

    fn image_code(&self) -> Option<&'static str> {
        self.set.as_ref()?.frames.iter().find_map(|f| f.observatory)
    }

    /// Fetches Gaia stars and (re)measures as needed; runs every frame.
    pub(super) fn poll_mpc(&mut self, ctx: &egui::Context) {
        // Gaia fetch.
        if let GaiaState::Fetching(f) = &self.mpc.gaia {
            match f.rx.try_recv() {
                Ok(Ok(fetched)) => {
                    if let Origin::Mirror(i) = fetched.origin {
                        self.prefs.gaia_last_good = i;
                    }
                    self.mpc.gaia = GaiaState::Ready {
                        stars: fetched.stars,
                        origin: fetched.origin,
                    };
                }
                Ok(Err(e)) => self.mpc.gaia = GaiaState::Failed(e),
                Err(TryRecvError::Disconnected) => {
                    self.mpc.gaia =
                        GaiaState::Failed("The Gaia download stopped unexpectedly.".into());
                }
                Err(TryRecvError::Empty) => {
                    // The elapsed seconds in the panel, at most once a second.
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                }
            }
        }
        // Measurement.
        if let Some(job) = &self.mpc.job {
            match job.rx.try_recv() {
                Ok(Ok(m)) => {
                    self.mpc.measured = Some(m);
                    self.mpc.error = None;
                    self.mpc.job = None;
                }
                Ok(Err(e)) => {
                    self.mpc.error = Some(e);
                    self.mpc.job = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.mpc.error = Some("Measuring stopped unexpectedly.".into());
                    self.mpc.job = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Some(set) = &self.set else { return };
        if self.mpc.forced.is_some() {
            return;
        }
        let has_objects = self.objects.iter().any(|o| o.marked() >= 2);
        if self.marking.is_some() || !(has_objects || self.mpc.review_open) {
            return;
        }
        if matches!(self.mpc.gaia, GaiaState::Idle) {
            match gaia::field(&set.frames) {
                Some(field) => {
                    let choice = self.prefs.gaia_mirror.checked_sub(1);
                    let order = gaia::order(choice, self.prefs.gaia_last_good);
                    self.mpc.gaia = GaiaState::Fetching(gaia::start(ctx, field, order));
                }
                None => {
                    self.mpc.gaia = GaiaState::Failed(
                        "The images have no sky coordinates (WCS) to look up Gaia stars with."
                            .into(),
                    );
                }
            }
        }
        let catalogue = match &self.mpc.gaia {
            GaiaState::Ready { stars, .. } => Some(stars.clone()),
            _ => None,
        };
        let names: Vec<String> = set.frames.iter().map(|f| f.name.clone()).collect();
        let (marks, _) = measure::marks_by_frame(&self.objects, names.len());
        let wanted = measure::key(&marks, catalogue.is_some());
        let current = self.mpc.measured.as_ref().map(|m| m.key);
        if self.mpc.job.is_none() && current != Some(wanted) && self.mpc.error.is_none() {
            self.mpc.job = Some(measure::start(
                ctx,
                set.paths.clone(),
                names,
                &self.objects,
                catalogue,
            ));
        }
    }

    /// Tries Gaia and measuring again (after a failure).
    fn retry_mpc(&mut self) {
        if let GaiaState::Fetching(f) = &self.mpc.gaia {
            f.cancel();
        }
        self.mpc.gaia = GaiaState::Idle;
        self.mpc.error = None;
        self.mpc.measured = None;
        self.mpc.forced = None;
    }

    /// Whether the measurement in hand is for the current marks.
    fn measured_is_current(&self) -> bool {
        let (Some(set), Some(m)) = (&self.set, &self.mpc.measured) else {
            return false;
        };
        let (marks, _) = measure::marks_by_frame(&self.objects, set.frames.len());
        let gaia = matches!(self.mpc.gaia, GaiaState::Ready { .. });
        m.key == measure::key(&marks, gaia)
    }

    /// Frames with marks whose plate solution failed or is poor, as text.
    fn solve_failures(&self) -> Vec<String> {
        let Some(m) = &self.mpc.measured else {
            return Vec::new();
        };
        if !m.solved {
            return Vec::new();
        }
        m.frames
            .iter()
            .enumerate()
            .filter(|(i, _)| m.owners.get(*i).is_some_and(|o| !o.is_empty()))
            .filter_map(|(i, f)| {
                let why = match f.solution.as_ref()? {
                    Err(e) => e.to_string(),
                    Ok(p) => p.problem()?,
                };
                Some(format!("Frame {}: {why}.", i + 1))
            })
            .collect()
    }

    /// The report as it stands, if there is a set.
    fn built(&self) -> Option<Built> {
        self.set.as_ref()?;
        let record = self.searches.record.as_ref();
        let prefix = record
            .map(|r| r.desig_prefix.trim())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| self.report_config.prefix())
            .to_owned();
        let start = record.map_or(1, |r| r.desig_start.max(1));
        let measured_by = self.measured_by();
        let current = self.measured_is_current();
        let failures = self.solve_failures();
        let measured = self
            .mpc
            .measured
            .as_ref()
            .filter(|_| current)
            .map(|m| report::Measured {
                frames: &m.frames,
                owners: &m.owners,
                solved: m.solved && failures.is_empty(),
            });
        Some(report::build(&Inputs {
            config: &self.report_config,
            measured_by: &measured_by,
            image_code: self.image_code(),
            objects: &self.objects,
            measured,
            prefix: &prefix,
            start,
        }))
    }

    fn state(&self, built: Option<&Built>) -> State {
        if let Some(f) = self.mpc.forced {
            return match f {
                Forced::Fetching => State::Fetching {
                    mirror: MIRRORS[1].name,
                    seconds: 3,
                },
                Forced::Measuring => State::Measuring,
                Forced::Offline => State::Offline("Couldn't reach any VizieR mirror.".into()),
                Forced::Failed => {
                    State::SolveFailed("Frame 2: 5 stars matched to Gaia, 8 needed.".into())
                }
                Forced::Preview => State::Preview,
                Forced::Missing => State::Missing(vec!["observers", "who measured"]),
                Forced::Ready => State::Ready { warnings: 0 },
            };
        }
        if self.set.is_none() {
            return State::NoSet;
        }
        if self.marking.is_some() {
            return State::Marking;
        }
        if !self.objects.iter().any(|o| o.marked() >= 2) {
            return State::NoObjects;
        }
        match &self.mpc.gaia {
            GaiaState::Fetching(f) => {
                return State::Fetching {
                    mirror: f.mirror_name(),
                    seconds: f.started.elapsed().as_secs(),
                };
            }
            GaiaState::Failed(e) => return State::Offline(e.clone()),
            _ => {}
        }
        if let Some(e) = &self.mpc.error {
            return State::Offline(e.clone());
        }
        if self.mpc.job.is_some() || !self.measured_is_current() {
            return State::Measuring;
        }
        let failures = self.solve_failures();
        if !failures.is_empty() {
            return State::SolveFailed(failures.join(" "));
        }
        if !matches!(self.mpc.gaia, GaiaState::Ready { .. }) {
            return State::Preview;
        }
        let Some(b) = built else { return State::NoSet };
        if b.lines == 0 {
            let why = b
                .rows
                .iter()
                .find(|r| r.include && !r.check.reportable())
                .map_or("Include at least one object.".to_owned(), |r| {
                    format!("{}: {}.", r.label, r.check.text())
                });
            return State::Empty(why);
        }
        let missing = self
            .report_config
            .missing(&self.measured_by(), self.image_code());
        if !missing.is_empty() {
            return State::Missing(missing);
        }
        let warnings = b
            .rows
            .iter()
            .filter(|r| r.include && (r.check != Check::Ready || !r.notes.is_empty()))
            .count()
            + self.report_config.advice(&self.measured_by()).len();
        State::Ready { warnings }
    }

    // ------------------------------------------------------------------
    // Side panel

    /// "Report for IASC" in the side panel: one state line (the full
    /// story in its tooltip) and, once there is an object, Review & copy.
    pub(super) fn mpc_section(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let built = self.built();
        let state = self.state(built.as_ref());
        let (tone, text) = describe(&state);
        let mut fix = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S1;
            // Room for the link, so the line never pushes the panel wider.
            let link = match state {
                State::Fetching { .. } => Some(("Cancel", Fix::Cancel)),
                State::Offline(_) | State::SolveFailed(_) => Some(("Try again", Fix::Retry)),
                State::Missing(_) => Some(("Fill in…", Fix::Settings)),
                _ => None,
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some((label, f)) = link
                    && ui
                        .link(RichText::new(label).small().color(p.accent))
                        .clicked()
                {
                    fix = Some(f);
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    status_icon(ui, &tone);
                    ui.add(Label::new(RichText::new(short(&state)).color(p.text)).truncate())
                        .on_hover_text(text);
                });
            });
        });
        match fix {
            Some(Fix::Cancel) => {
                if let GaiaState::Fetching(f) = &self.mpc.gaia {
                    f.cancel();
                }
            }
            Some(Fix::Retry) => self.retry_mpc(),
            Some(Fix::Settings) => self.open_settings(Category::Report),
            None => {}
        }
        if matches!(state, State::NoSet | State::NoObjects | State::Marking) {
            return;
        }
        self.copied_line(ui, built.as_ref(), true);
        let primary = Button::new(RichText::new("Review & copy report").color(p.accent_text))
            .fill(p.accent)
            .stroke(Stroke::NONE)
            .min_size(vec2(ui.available_width(), theme::TOOL));
        if ui
            .add(primary)
            .on_hover_text(
                "Check each object, then copy the report to paste into the IASC report box",
            )
            .clicked()
        {
            self.mpc.review_open = true;
        }
    }

    /// `Copied 14:05 UTC — paste it into the IASC report box`, or that the
    /// report changed since; `compact` for one line in the panel.
    fn copied_line(&self, ui: &mut egui::Ui, built: Option<&Built>, compact: bool) {
        let p = palette(ui);
        let Some(r) = &self.searches.record else {
            return;
        };
        let Some(at) = r.mpc_copied else { return };
        let clock = crate::format::unix_clock(at);
        let (text, color) = if built.is_some_and(|b| b.mpc != r.mpc_report) {
            (
                format!("Changed since you copied it at {clock}: copy it again."),
                p.danger,
            )
        } else {
            (
                format!("Copied {clock} — paste it into the IASC report box."),
                p.muted,
            )
        };
        if compact {
            let line = if color == p.danger {
                format!("Changed since copied at {clock}")
            } else {
                format!("Copied {clock}")
            };
            ui.add(Label::new(RichText::new(line).small().color(color)).truncate())
                .on_hover_text(text);
        } else {
            ui.add(Label::new(RichText::new(text).small().color(color)).wrap());
        }
    }

    // ------------------------------------------------------------------
    // Review window

    pub(super) fn mpc_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.mpc.review_open {
            return;
        }
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 2.0 * S5).min(860.0),
            (screen.height() - 2.0 * S5).min(720.0),
        )
        .max(vec2(360.0, 300.0));
        let style = ctx.global_style();
        let modal = egui::Modal::new(Id::new("mpc_report"))
            .frame(
                Frame::popup(&style)
                    .inner_margin(Margin::same(S4 as i8))
                    .corner_radius(theme::RADIUS + 2),
            )
            .backdrop_color(Color32::from_black_alpha(70))
            .show(ctx, |ui| {
                ui.set_width(size.x - 2.0 * S4);
                ui.set_max_height(size.y - 2.0 * S4);
                self.mpc_body(ui, frame);
            });
        if modal.should_close() {
            self.mpc.review_open = false;
        }
    }

    fn mpc_body(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = palette(ui);
        ui.horizontal(|ui| {
            ui.label(RichText::new("MPC report").heading().color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icons::small_button(ui, Icon::Close, p.muted)
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    self.mpc.review_open = false;
                }
            });
        });
        ui.add_space(S2);
        let built = self.built();
        let state = self.state(built.as_ref());
        ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 3.0 * theme::TOOL)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = S2;
                let (tone, text) = describe(&state);
                status_line(ui, &tone, &text);
                self.source_lines(ui);
                ui.add_space(S1);
                self.search_fields(ui);
                ui.add_space(S2);
                if let Some(b) = &built {
                    self.object_table(ui, b);
                    ui.add_space(S2);
                    for a in self.report_config.advice(&self.measured_by()) {
                        ui.add(
                            Label::new(RichText::new(format!("• {a}")).small().color(p.muted))
                                .wrap(),
                        );
                    }
                    ui.add_space(S1);
                    report_text(ui, &b.mpc, matches!(state, State::Ready { .. }));
                }
            });
        ui.add_space(S2);
        self.mpc_actions(ui, frame, built.as_ref(), &state);
    }

    /// Where the positions come from: Gaia, and each frame's plate fit.
    fn source_lines(&self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let small = |ui: &mut egui::Ui, t: String, c: Color32| {
            ui.add(Label::new(RichText::new(t).small().color(c)).wrap());
        };
        match &self.mpc.gaia {
            GaiaState::Ready { stars, origin } => {
                let from = match origin {
                    Origin::Cache => "saved on this computer".to_owned(),
                    Origin::Mirror(i) => format!("from VizieR ({})", MIRRORS[*i].name),
                };
                small(
                    ui,
                    format!("Gaia DR3: {} stars, {from}.", stars.len()),
                    p.muted,
                );
            }
            GaiaState::Failed(e) => small(ui, e.clone(), p.danger),
            _ => {}
        }
        let Some(m) = self.mpc.measured.as_ref().filter(|m| m.solved) else {
            return;
        };
        for (i, f) in m.frames.iter().enumerate() {
            let line = match &f.solution {
                Some(Ok(s)) => {
                    let zp = f.zero_point.map_or(String::new(), |z| {
                        format!(", magnitudes from {} stars (±{:.2})", z.stars, z.scatter)
                    });
                    let mut t = format!(
                        "Frame {}: fit to {} Gaia stars, {:.2}″ RMS{zp}.",
                        i + 1,
                        s.stars,
                        (s.rms.0.powi(2) + s.rms.1.powi(2)).sqrt() / std::f64::consts::SQRT_2
                    );
                    if let Some(problem) = s.problem() {
                        t += &format!(" Not usable: {problem}.");
                    }
                    t
                }
                Some(Err(e)) => format!("Frame {}: not solved: {e}.", i + 1),
                None => continue,
            };
            small(ui, line, p.muted);
        }
    }

    /// Measured by, and the designations' prefix and first number.
    fn search_fields(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let default_prefix = self.report_config.prefix().to_owned();
        let Some(r) = &mut self.searches.record else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Measured by").color(p.text));
            let edit = ui
                .add(
                    TextEdit::singleline(&mut r.searched_by)
                        .hint_text("Initials and surname, e.g. A. Yılmaz")
                        .desired_width(220.0),
                )
                .on_hover_text("The MEA line. Several people: separate them with commas.");
            if edit.changed() {
                self.searches.config.name = r.searched_by.trim().to_owned();
                if self.capture.is_none() {
                    self.searches.config.save();
                }
            }
            ui.add_space(S3);
            ui.label(RichText::new("Names").color(p.text));
            ui.add(
                TextEdit::singleline(&mut r.desig_prefix)
                    .hint_text(default_prefix.as_str())
                    .desired_width(64.0)
                    .char_limit(6),
            )
            .on_hover_text("Designation prefix (default in Settings → Observer & report)");
            ui.label(RichText::new("from").color(p.muted));
            let mut start = r.desig_start.max(1);
            if ui
                .add(egui::DragValue::new(&mut start).range(1..=99_999))
                .on_hover_text("Number of the first object; the others count up from it")
                .changed()
            {
                r.desig_start = start;
            }
        });
        if let Some(problem) =
            mpc::name_problem(&self.measured_by()).filter(|_| !self.measured_by().is_empty())
        {
            ui.add(Label::new(RichText::new(problem).small().color(p.danger)).wrap());
        }
    }

    /// One row per object: include, name, frames, track fit, magnitude,
    /// known object, check, discovery.
    fn object_table(&mut self, ui: &mut egui::Ui, b: &Built) {
        let p = palette(ui);
        if b.rows.is_empty() {
            muted_wrapped(ui, "No objects marked yet.");
            return;
        }
        let mut toggles: Vec<(usize, bool, bool)> = Vec::new();
        egui::Grid::new("mpc_objects")
            .striped(true)
            .spacing(vec2(S3, S1))
            .show(ui, |ui| {
                for h in [
                    "Include",
                    "Object",
                    "Name",
                    "Frames",
                    "Track",
                    "Mag",
                    "Known",
                    "Check",
                    "Discovery",
                ] {
                    ui.label(RichText::new(h).small().color(p.muted));
                }
                ui.end_row();
                for r in &b.rows {
                    let mut include = r.include;
                    ui.checkbox(&mut include, "")
                        .on_hover_text("Put it in the report");
                    ui.label(RichText::new(&r.label).color(p.tag));
                    ui.label(RichText::new(&r.designation).monospace().color(p.text));
                    ui.label(RichText::new(format!("{}/{}", r.measured, r.marked)).color(p.text))
                        .on_hover_text("Frames measured / frames marked");
                    ui.label(
                        RichText::new(r.track_rms.map_or("–".into(), |t| format!("{t:.2}″")))
                            .color(p.text),
                    )
                    .on_hover_text(
                        "How far the positions stray from a straight, steady track (RMS)",
                    );
                    ui.label(
                        RichText::new(r.mag.map_or("–".into(), |m| format!("{m:.1} G")))
                            .color(p.text),
                    );
                    ui.label(
                        RichText::new(r.known.as_deref().unwrap_or("–"))
                            .small()
                            .color(p.known),
                    );
                    let (icon, color) = match r.check {
                        Check::Ready => (Icon::Check, p.success),
                        Check::Waiting => (Icon::History, p.muted),
                        Check::Unsnapped(_) => (Icon::Info, p.warning),
                        _ => (Icon::Close, p.danger),
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = S1;
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                        icons::paint(ui.painter(), rect, icon, color);
                        let mut label =
                            ui.label(RichText::new(r.check.text()).small().color(p.text));
                        if !r.notes.is_empty() {
                            label = label.on_hover_text(r.notes.join("\n"));
                        }
                        let _ = label;
                    });
                    let mut discovery = r.discovery;
                    ui.checkbox(&mut discovery, "")
                        .on_hover_text("A new object you found: “*” on its first line");
                    ui.end_row();
                    if include != r.include || discovery != r.discovery {
                        toggles.push((r.index, include, discovery));
                    }
                }
            });
        for (i, include, discovery) in toggles {
            if let Some(o) = self.objects.get_mut(i) {
                o.exclude = !include;
                o.discovery = discovery;
            }
        }
        let notes: Vec<String> = b
            .rows
            .iter()
            .flat_map(|r| r.notes.iter().map(move |n| format!("{}: {n}", r.label)))
            .collect();
        for n in notes {
            ui.add(Label::new(RichText::new(n).small().color(p.muted)).wrap());
        }
    }

    /// Copy, save and export.
    fn mpc_actions(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        built: Option<&Built>,
        state: &State,
    ) {
        let p = palette(ui);
        let blocked: Option<String> = match state {
            State::Ready { .. } => None,
            State::Preview if report::ALLOW_PREVIEW_COPY => None,
            State::Preview | State::Offline(_) => {
                Some("Can't copy: no Gaia plate solution yet".into())
            }
            State::SolveFailed(_) => Some("Can't copy: a frame didn't solve".into()),
            State::Missing(m) => Some(format!("Can't copy: add {}", m.join(", "))),
            State::Measuring | State::Fetching { .. } => Some("Measuring…".into()),
            State::Empty(_) => Some("Nothing ready to report".into()),
            _ => Some("Mark an object first".into()),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            let text = blocked.clone().unwrap_or_else(|| "Copy report".into());
            let copy = Button::new(RichText::new(text).color(if blocked.is_none() { p.accent_text } else { p.muted }))
                .fill(if blocked.is_none() { p.accent } else { p.inset })
                .stroke(Stroke::NONE)
                .min_size(vec2(240.0, theme::TOOL + 4.0));
            if ui
                .add_enabled(blocked.is_none(), copy)
                .on_hover_text("Copies the report; paste it into the IASC report box")
                .clicked()
                && let Some(b) = built
            {
                ui.ctx().copy_text(b.mpc.clone());
                self.mark_reported();
                if let Some(r) = &mut self.searches.record {
                    r.mpc_report = b.mpc.clone();
                    r.mpc_copied = Some(crate::format::unix_now());
                }
            }
            let can_save = built.is_some_and(|b| b.lines > 0) && blocked.is_none();
            if ui
                .add_enabled(can_save, Button::new("Save .txt…"))
                .on_hover_text("The same report as a text file")
                .clicked()
                && let Some(b) = built
            {
                self.save_text(frame, &b.mpc, "txt", "MPC report");
            }
            if ui
                .add_enabled(can_save, Button::new("Save ADES…"))
                .on_hover_text("The report in the MPC's newer ADES format (pipe-separated, .psv), with each position's uncertainty")
                .clicked()
                && let Some(b) = built
            {
                self.save_text(frame, &b.ades, "psv", "ADES report");
            }
            let can_export = self.searches.record.is_some() && self.export.is_none();
            if ui
                .add_enabled(can_export, Button::new("Export as JSON…"))
                .on_hover_text("Your search with each object's positions and this report")
                .clicked()
            {
                self.mpc.review_open = false;
                self.export_json(ui.ctx(), frame, false);
            }
        });
        if let Some(r) = &self.searches.record
            && r.mpc_copied.is_some()
        {
            self.copied_line(ui, built, false);
        }
        if let Some(n) = &self.mpc.note {
            ui.label(RichText::new(n).small().color(p.muted));
        }
    }

    fn save_text(&mut self, frame: &eframe::Frame, text: &str, ext: &str, what: &str) {
        let set = self
            .set
            .as_ref()
            .map_or_else(String::new, |s| s.name.clone());
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("Save {what}"))
            .set_file_name(format!("{set} {what}.{ext}"))
            .add_filter(what, &[ext])
            .set_parent(frame)
            .save_file()
        else {
            return;
        };
        self.mpc.note = Some(match std::fs::write(&path, text) {
            Ok(()) => {
                update::reveal(&path);
                format!(
                    "Saved {}",
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                )
            }
            Err(e) => format!("Couldn't save: {e}"),
        });
    }
}

fn describe(state: &State) -> (Tone, String) {
    match state {
        State::NoSet => (Tone::Idle, "Open an image set first.".into()),
        State::NoObjects => (
            Tone::Idle,
            format!(
                "Mark a moving object on at least {} frames to make a report.",
                report::MIN_FRAMES
            ),
        ),
        State::Marking => (Tone::Idle, "Finish marking the object first.".into()),
        State::Fetching { mirror, seconds } => (
            Tone::Busy,
            format!("Getting Gaia stars from {mirror}… {seconds} s"),
        ),
        State::Measuring => (Tone::Busy, "Measuring positions…".into()),
        State::Offline(e) => (
            Tone::Blocked,
            format!("{e} Without Gaia stars the positions are only a preview."),
        ),
        State::SolveFailed(e) => (
            Tone::Blocked,
            format!("Couldn't match the stars to Gaia. {e}"),
        ),
        State::Preview => (
            Tone::Blocked,
            "Preview: positions from the image header, several arcseconds off. Not precise \
             enough to submit until the frames are matched to Gaia."
                .into(),
        ),
        State::Missing(m) => (
            Tone::Blocked,
            format!("Add {} before copying.", m.join(", ")),
        ),
        State::Empty(why) => (Tone::Warn, format!("Nothing to report yet. {why}")),
        State::Ready { warnings: 0 } => (Tone::Good, "Ready to copy.".into()),
        State::Ready { warnings } => (
            Tone::Warn,
            format!(
                "Ready, with {warnings} thing{} to check.",
                if *warnings == 1 { "" } else { "s" }
            ),
        ),
    }
}

/// An icon (its shape tells the state, so colour is not needed) and text.
fn status_line(ui: &mut egui::Ui, tone: &Tone, text: &str) {
    let p = palette(ui);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        status_icon(ui, tone);
        ui.add(Label::new(RichText::new(text).color(p.text)).wrap());
    });
}

fn status_icon(ui: &mut egui::Ui, tone: &Tone) {
    let p = palette(ui);
    let (icon, color) = match tone {
        Tone::Busy => {
            ui.add(egui::Spinner::new().size(14.0).color(p.muted));
            return;
        }
        Tone::Idle => (Icon::Info, p.muted),
        Tone::Good => (Icon::Check, p.success),
        Tone::Warn => (Icon::Warning, p.warning),
        Tone::Blocked => (Icon::Close, p.danger),
    };
    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
    icons::paint(ui.painter(), rect, icon, color);
}

/// What the link after the panel's state line does.
#[derive(Clone, Copy)]
enum Fix {
    Cancel,
    Retry,
    Settings,
}

/// The state in a few words, for one line in the panel; [`describe`] has
/// the whole story for its tooltip.
fn short(state: &State) -> String {
    match state {
        State::NoSet => "Open an image set first".into(),
        State::NoObjects => "Mark an object first".into(),
        State::Marking => "Finish marking first".into(),
        State::Fetching { seconds, .. } => format!("Getting Gaia stars… {seconds} s"),
        State::Measuring => "Measuring positions…".into(),
        State::Offline(_) => "No Gaia stars: preview only".into(),
        State::SolveFailed(_) => "Couldn't match to Gaia".into(),
        State::Preview => "Preview only, not precise".into(),
        State::Missing(_) => "Observer details missing".into(),
        State::Empty(_) => "Nothing to report yet".into(),
        State::Ready { warnings: 0 } => "Ready to copy".into(),
        State::Ready { warnings } => format!("Ready · {warnings} to check"),
    }
}

/// The report in monospace, selectable, marked as a preview when it can't
/// be submitted.
fn report_text(ui: &mut egui::Ui, text: &str, ready: bool) {
    let p = palette(ui);
    if !ready {
        ui.label(
            RichText::new("PREVIEW — not ready to submit")
                .small()
                .family(theme::semibold())
                .color(p.danger),
        );
    }
    Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::same(S2 as i8))
        .show(ui, |ui| {
            ScrollArea::horizontal().id_salt("mpc_text").show(ui, |ui| {
                ui.add(
                    Label::new(
                        RichText::new(text)
                            .font(FontId::monospace(12.0))
                            .color(p.text),
                    )
                    .selectable(true)
                    .extend(),
                );
            });
        });
}

// ---------------------------------------------------------------------------
// Settings rows (Observer & report; Known objects → mirrors)

fn text_row(
    app: &mut App,
    ui: &mut egui::Ui,
    hint: &str,
    width: f32,
    get: fn(&mut report::Config) -> &mut String,
) {
    let edit = ui.add(
        TextEdit::singleline(get(&mut app.report_config))
            .hint_text(hint)
            .desired_width(width),
    );
    if edit.changed() && app.capture.is_none() {
        app.report_config.save();
    }
}

pub fn code_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let from_images = app.image_code();
    ui.horizontal(|ui| {
        text_row(app, ui, from_images.unwrap_or("e.g. F51"), 80.0, |c| {
            &mut c.code
        });
        if let Some(c) = from_images {
            ui.label(
                RichText::new(format!("The open images: {c}"))
                    .small()
                    .color(p.muted),
            );
        }
    });
}

pub fn observers_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    text_row(app, ui, "e.g. J. R. Smith, A. Jones", 360.0, |c| {
        &mut c.observers
    });
    for n in app
        .report_config
        .observers
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        if let Some(problem) = mpc::name_problem(n) {
            ui.label(RichText::new(problem).small().color(p.danger));
        }
    }
}

pub fn contact_setting(app: &mut App, ui: &mut egui::Ui) {
    text_row(app, ui, "Name, e.g. A. Teacher", 360.0, |c| {
        &mut c.contact_name
    });
    text_row(
        app,
        ui,
        "Address, e.g. School, Street 1, City, Country",
        360.0,
        |c| &mut c.contact_address,
    );
    text_row(app, ui, "E-mail", 360.0, |c| &mut c.contact_email);
}

pub fn telescope_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    ui.horizontal_wrapped(|ui| {
        text_row(app, ui, "Aperture (m)", 90.0, |c| &mut c.aperture);
        text_row(app, ui, "f-ratio", 60.0, |c| &mut c.f_ratio);
        text_row(app, ui, "Design, e.g. reflector", 160.0, |c| &mut c.design);
        text_row(app, ui, "CCD", 60.0, |c| &mut c.detector);
    });
    let line = app.report_config.telescope().tel_line();
    if !line.is_empty() {
        ui.label(
            RichText::new(format!("TEL {line}"))
                .monospace()
                .small()
                .color(p.muted),
        );
    }
}

pub fn prefix_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        text_row(app, ui, report::DEFAULT_PREFIX, 80.0, |c| &mut c.prefix);
        let first = report::designation(app.report_config.prefix(), 1);
        ui.label(
            RichText::new(format!("First object: {first}"))
                .small()
                .color(p.muted),
        );
    });
    if let Some(problem) =
        mpc::designation_problem(&report::designation(app.report_config.prefix(), 1))
    {
        ui.label(
            RichText::new(format!("Designation: {problem}"))
                .small()
                .color(p.danger),
        );
    }
}

pub fn ack_setting(app: &mut App, ui: &mut egui::Ui) {
    text_row(app, ui, "ACK text, e.g. IASC campaign batch", 360.0, |c| {
        &mut c.ack
    });
    text_row(app, ui, "AC2 e-mails, comma-separated", 360.0, |c| {
        &mut c.ack_emails
    });
}

/// The header lines as they'll be sent, updated as the fields change.
pub fn header_preview(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let measured_by = app.searches.config.name.clone();
    let details = app
        .report_config
        .details(&measured_by, app.image_code().or(Some("F51")));
    let text = mpc::header_lines(&details).join("\n");
    Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(theme::RADIUS)
        .inner_margin(Margin::same(S2 as i8))
        .show(ui, |ui| {
            ui.add(
                Label::new(
                    RichText::new(text)
                        .font(FontId::monospace(12.0))
                        .color(p.text),
                )
                .selectable(true),
            );
        });
    let missing = app
        .report_config
        .missing(&measured_by, app.image_code().or(Some("F51")));
    if !missing.is_empty() {
        ui.label(
            RichText::new(format!(
                "Still needed before a report can be copied: {}.",
                missing.join(", ")
            ))
            .small()
            .color(p.danger),
        );
    }
}

/// Each mirror with a dot and a word (OK 0.8 s / Slow / Failed), and Test all.
pub fn mirror_status(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    if let Some(rx) = &app.mirror_tests.rx {
        while let Ok((i, r)) = rx.try_recv() {
            app.mirror_tests.results[i] = Some(r);
        }
        if app.mirror_tests.results.iter().all(Option::is_some) {
            app.mirror_tests.rx = None;
        }
    }
    let testing = app.mirror_tests.rx.is_some();
    egui::Grid::new("mirrors")
        .spacing(vec2(S3, S1))
        .show(ui, |ui| {
            for (i, m) in MIRRORS.iter().enumerate() {
                let (color, word, filled) = match &app.mirror_tests.results[i] {
                    None if testing => (p.muted, "Testing…".to_owned(), false),
                    None => (p.muted, "Not tested".to_owned(), false),
                    Some(Ok(d)) if *d > gaia::SLOW => {
                        (p.warning, format!("Slow {:.1} s", d.as_secs_f64()), true)
                    }
                    Some(Ok(d)) => (p.success, format!("OK {:.1} s", d.as_secs_f64()), true),
                    Some(Err(_)) => (p.danger, "Failed".to_owned(), false),
                };
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
                    if filled {
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                    } else {
                        ui.painter()
                            .circle_stroke(rect.center(), 3.5, Stroke::new(1.5, color));
                    }
                    ui.label(RichText::new(m.name).color(p.text));
                });
                let label = ui.label(RichText::new(word).small().color(p.text));
                if let Some(Err(e)) = &app.mirror_tests.results[i] {
                    label.on_hover_text(e);
                }
                if app.prefs.gaia_last_good == i {
                    ui.label(RichText::new("last used").small().color(p.muted));
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    if ui.add_enabled(!testing, Button::new("Test all")).clicked() {
        app.mirror_tests.results = Default::default();
        app.mirror_tests.rx = Some(gaia::test_all(ui.ctx()));
    }
}
