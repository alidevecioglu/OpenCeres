//! The team server in the window: signing in (Settings → Team), the Team
//! window (assignments and announcements), and, for a set that belongs to
//! one of the user's campaigns, its badge, "Submit review" and everyone
//! else's findings drawn like imported reports. The protocol is in
//! [`crate::team`].
//!
//! Every request runs on a thread of its own and comes back through one
//! channel, with a repaint; nothing polls the server. Lists are fetched
//! when the Team window opens or on Refresh, a set's matches and findings
//! when it opens.

use std::sync::mpsc::{Receiver, Sender};

use eframe::egui::{
    self, Align, Button, Color32, Frame, Id, Key, Label, Layout, Margin, RichText, Stroke,
    TextEdit, vec2,
};

use super::{App, hline, muted_wrapped, section_heading};
use crate::format;
use crate::history::Record;
use crate::icons::{self, Icon};
use crate::settings::Category;
use crate::team::{
    self, Announcement, ApiError, Assignment, Match, Me, ServerInfo, Submissions, Submitted, User,
};
use crate::theme::{self, S1, S2, S3, S4, S5, palette};

const WINDOW: [f32; 2] = [560.0, 520.0];
const FIELD: f32 = 280.0;

/// A finished request.
enum Event {
    Tested(Result<ServerInfo, ApiError>),
    SignedIn(Result<(String, User), ApiError>),
    Me(Result<Me, ApiError>),
    Password(Result<(), ApiError>),
    Assignments(Result<Vec<Assignment>, ApiError>),
    Announcements(Result<Vec<Announcement>, ApiError>),
    /// `set` is the [`Team::set_gen`] the request was made for.
    Lookup(u64, Result<Vec<Match>, ApiError>),
    Findings(u64, Result<Submissions, ApiError>),
    Submitted(u64, Result<Submitted, ApiError>),
    /// Nothing to do (signing out).
    Done,
}

pub(super) struct Team {
    pub config: team::Config,
    /// Off for screenshots, which must not depend on (or touch) a server.
    enabled: bool,
    ctx: egui::Context,
    tx: Sender<(u64, Event)>,
    rx: Receiver<(u64, Event)>,
    /// Bumped on sign in / out and address changes, so late answers for
    /// the old session are dropped.
    session: u64,
    /// Bumped whenever a set opens, for the same reason.
    set_gen: u64,

    // Settings → Team
    url: String,
    url_error: Option<String>,
    testing: bool,
    tested: Option<Result<String, String>>,
    email: String,
    password: String,
    signing_in: bool,
    account_error: Option<String>,
    account_note: Option<String>,
    me: Option<Me>,
    /// The server wants a new password before anything else.
    must_change: bool,
    password_open: bool,
    current_pw: String,
    new_pw: String,
    repeat_pw: String,
    changing: bool,
    password_error: Option<String>,

    // The Team window
    pub window_open: bool,
    lists_pending: u8,
    assignments: Option<Vec<Assignment>>,
    announcements: Option<Vec<Announcement>>,
    lists_error: Option<String>,

    // The open set
    files: Vec<String>,
    looking_up: bool,
    lookup_error: Option<String>,
    pub matched: Option<Match>,
    fetching: bool,
    hidden: bool,
    /// Ids of the reports shown from the server.
    shared: Vec<String>,
    findings_error: Option<String>,
    submitting: bool,
    submit_note: Option<String>,
    submit_error: Option<String>,
}

impl Team {
    pub fn new(ctx: &egui::Context, enabled: bool) -> Team {
        let config = if enabled {
            team::Config::load()
        } else {
            team::Config::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        Team {
            url: config.url.clone(),
            email: config.email.clone(),
            config,
            enabled,
            ctx: ctx.clone(),
            tx,
            rx,
            session: 0,
            set_gen: 0,
            url_error: None,
            testing: false,
            tested: None,
            password: String::new(),
            signing_in: false,
            account_error: None,
            account_note: None,
            me: None,
            must_change: false,
            password_open: false,
            current_pw: String::new(),
            new_pw: String::new(),
            repeat_pw: String::new(),
            changing: false,
            password_error: None,
            window_open: false,
            lists_pending: 0,
            assignments: None,
            announcements: None,
            lists_error: None,
            files: Vec::new(),
            looking_up: false,
            lookup_error: None,
            matched: None,
            fetching: false,
            hidden: false,
            shared: Vec::new(),
            findings_error: None,
            submitting: false,
            submit_note: None,
            submit_error: None,
        }
    }

    pub fn configured(&self) -> bool {
        !self.config.url.is_empty()
    }

    /// Signed in and allowed to use the server (no password change due).
    fn ready(&self) -> bool {
        self.config.signed_in() && !self.must_change
    }

    fn spawn(&self, job: impl FnOnce() -> Event + Send + 'static) {
        if !self.enabled {
            return;
        }
        let (tx, ctx, session) = (self.tx.clone(), self.ctx.clone(), self.session);
        std::thread::spawn(move || {
            let _ = tx.send((session, job()));
            ctx.request_repaint();
        });
    }

    fn save(&self) {
        if self.enabled {
            self.config.save();
        }
    }

    /// Forgets the session (and everything fetched with it).
    fn forget_session(&mut self) {
        self.config.sign_out();
        self.save();
        self.session += 1;
        self.me = None;
        self.must_change = false;
        self.password_open = false;
        self.signing_in = false;
        self.changing = false;
        self.lists_pending = 0;
        self.assignments = None;
        self.announcements = None;
        self.lists_error = None;
        self.looking_up = false;
        self.lookup_error = None;
        self.matched = None;
        self.fetching = false;
        self.hidden = false;
        self.submitting = false;
        self.submit_note = None;
        self.submit_error = None;
        self.findings_error = None;
    }

    /// What a failed request means for the session; the message to show.
    fn failed(&mut self, e: ApiError) -> String {
        match &e {
            ApiError::SignedOut(_) => {
                self.forget_session();
                self.account_error = Some("You were signed out. Please sign in again.".into());
            }
            ApiError::MustChangePassword => self.must_change = true,
            ApiError::Other(_) => {}
        }
        e.message()
    }
}

/// `Open` for `open`; the server's statuses are single words.
fn capitalized(s: &str) -> String {
    let s = s.replace('_', " ");
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

fn small(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    ui.add(Label::new(RichText::new(text.into()).small().color(color)).wrap());
}

fn spinner_line(ui: &mut egui::Ui, text: &str) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(13.0).color(p.muted));
        ui.label(RichText::new(text).small().color(p.muted));
    });
}

fn primary(ui: &egui::Ui, text: &str) -> Button<'static> {
    let p = palette(ui);
    Button::new(RichText::new(text.to_owned()).color(p.accent_text))
        .fill(p.accent)
        .stroke(Stroke::NONE)
}

impl App {
    /// At start: check the saved session once (who, which teams).
    pub(super) fn team_start(&mut self) {
        if self.team.config.signed_in() {
            let (url, token) = (self.team.config.url.clone(), self.team.config.token.clone());
            self.team.spawn(move || Event::Me(team::me(&url, &token)));
        }
    }

    /// A set has opened: ask whether it belongs to one of the user's campaigns.
    pub(super) fn team_set_opened(&mut self) {
        let t = &mut self.team;
        t.set_gen += 1;
        t.files = self
            .set
            .as_ref()
            .map(|s| s.frames.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default();
        t.matched = None;
        t.lookup_error = None;
        t.looking_up = false;
        t.fetching = false;
        t.hidden = false;
        t.shared.clear();
        t.findings_error = None;
        t.submitting = false;
        t.submit_note = None;
        t.submit_error = None;
        self.team_lookup();
    }

    fn team_lookup(&mut self) {
        let t = &mut self.team;
        if !t.ready() || t.files.is_empty() || t.looking_up {
            return;
        }
        t.looking_up = true;
        t.lookup_error = None;
        let (url, token, files, set) = (
            t.config.url.clone(),
            t.config.token.clone(),
            t.files.clone(),
            t.set_gen,
        );
        t.spawn(move || Event::Lookup(set, team::lookup(&url, &token, &files)));
    }

    /// Everyone's reports of the matched set.
    fn team_fetch_findings(&mut self) {
        let t = &mut self.team;
        let Some(m) = &t.matched else { return };
        if !t.ready() || t.fetching {
            return;
        }
        t.fetching = true;
        t.findings_error = None;
        let (url, token, id, set) = (
            t.config.url.clone(),
            t.config.token.clone(),
            m.set_id.clone(),
            t.set_gen,
        );
        t.spawn(move || Event::Findings(set, team::submissions(&url, &token, &id)));
    }

    /// Sends the detailed report (as exported, without cutouts).
    fn team_submit(&mut self) {
        let Some(record) = self.report_record() else {
            return;
        };
        let t = &mut self.team;
        let Some(m) = &t.matched else { return };
        if !t.ready() || t.submitting {
            return;
        }
        t.submitting = true;
        t.submit_note = None;
        t.submit_error = None;
        let (url, token, id, set) = (
            t.config.url.clone(),
            t.config.token.clone(),
            m.set_id.clone(),
            t.set_gen,
        );
        let body = record.to_json("");
        t.spawn(move || Event::Submitted(set, team::submit(&url, &token, &id, &body)));
    }

    fn team_refresh_lists(&mut self) {
        let t = &mut self.team;
        if !t.ready() || t.lists_pending > 0 {
            return;
        }
        t.lists_pending = 3;
        t.lists_error = None;
        let (url, token) = (t.config.url.clone(), t.config.token.clone());
        let (u2, k2, u3, k3) = (url.clone(), token.clone(), url.clone(), token.clone());
        t.spawn(move || Event::Me(team::me(&url, &token)));
        t.spawn(move || Event::Assignments(team::assignments(&u2, &k2)));
        t.spawn(move || Event::Announcements(team::announcements(&u3, &k3)));
    }

    pub(super) fn toggle_team(&mut self) {
        self.team.window_open = !self.team.window_open;
        if self.team.window_open {
            self.settings_open = false;
            self.searches.window_open = false;
            self.help_open = false;
            self.team_refresh_lists();
        }
    }

    pub(super) fn poll_team(&mut self) {
        while let Ok((session, event)) = self.team.rx.try_recv() {
            if session == self.team.session {
                self.team_event(event);
            }
        }
    }

    fn team_event(&mut self, event: Event) {
        let t = &mut self.team;
        match event {
            Event::Done => {}
            Event::Tested(result) => {
                t.testing = false;
                t.tested = Some(
                    result
                        .map(|i| {
                            let version = if i.version.is_empty() {
                                String::new()
                            } else {
                                format!(" {}", i.version)
                            };
                            format!("Connected: OpenCeres Team{version}")
                        })
                        .map_err(|e| e.message()),
                );
            }
            Event::SignedIn(result) => {
                t.signing_in = false;
                match result {
                    Ok((token, user)) => {
                        t.config.token = token;
                        t.config.name = user.name.clone();
                        t.config.email = if user.email.is_empty() {
                            t.email.trim().to_owned()
                        } else {
                            user.email.clone()
                        };
                        t.save();
                        t.must_change = user.must_change_password;
                        if t.must_change {
                            // The temporary password they just typed.
                            t.current_pw = std::mem::take(&mut t.password);
                        }
                        t.password.clear();
                        t.account_error = None;
                        t.account_note = None;
                        self.team_start();
                        self.team_lookup();
                        if self.team.window_open {
                            self.team_refresh_lists();
                        }
                    }
                    Err(e) => t.account_error = Some(e.message()),
                }
            }
            Event::Me(result) => {
                t.lists_pending = t.lists_pending.saturating_sub(1);
                match result {
                    Ok(me) => {
                        t.must_change |= me.user.must_change_password;
                        if !me.user.name.is_empty() && me.user.name != t.config.name {
                            t.config.name = me.user.name.clone();
                            t.save();
                        }
                        t.me = Some(me);
                    }
                    Err(e) => {
                        let m = t.failed(e);
                        t.lists_error.get_or_insert(m);
                    }
                }
            }
            Event::Password(result) => {
                t.changing = false;
                match result {
                    Ok(()) => {
                        t.must_change = false;
                        t.password_open = false;
                        t.current_pw.clear();
                        t.new_pw.clear();
                        t.repeat_pw.clear();
                        t.password_error = None;
                        t.account_note = Some("Password changed.".into());
                        self.team_start();
                        self.team_lookup();
                        if self.team.window_open {
                            self.team_refresh_lists();
                        }
                    }
                    Err(ApiError::MustChangePassword) => {
                        t.password_error = Some("The server didn't accept that.".into());
                    }
                    Err(e) => t.password_error = Some(t.failed(e)),
                }
            }
            Event::Assignments(result) => {
                t.lists_pending = t.lists_pending.saturating_sub(1);
                match result {
                    Ok(list) => t.assignments = Some(list),
                    Err(e) => {
                        let m = t.failed(e);
                        t.lists_error.get_or_insert(m);
                    }
                }
            }
            Event::Announcements(result) => {
                t.lists_pending = t.lists_pending.saturating_sub(1);
                match result {
                    Ok(list) => t.announcements = Some(list),
                    Err(e) => {
                        let m = t.failed(e);
                        t.lists_error.get_or_insert(m);
                    }
                }
            }
            Event::Lookup(set, result) => {
                if set != t.set_gen {
                    return;
                }
                t.looking_up = false;
                match result {
                    Ok(matches) => {
                        t.matched = team::pick_match(&matches).cloned();
                        self.team_fetch_findings();
                    }
                    Err(e) => t.lookup_error = Some(t.failed(e)),
                }
            }
            Event::Findings(set, result) => {
                if set != t.set_gen {
                    return;
                }
                t.fetching = false;
                match result {
                    Ok(s) => self.show_findings(s),
                    Err(e) => t.findings_error = Some(t.failed(e)),
                }
            }
            Event::Submitted(set, result) => {
                if set != t.set_gen {
                    return;
                }
                t.submitting = false;
                match result {
                    Ok(done) => {
                        if let Some(m) = t.matched.as_mut().filter(|m| m.assigned) {
                            m.status = Some("submitted".into());
                        }
                        let at = done.submitted_at.unwrap_or_else(format::unix_now);
                        t.submit_note = Some(format!("Submitted at {}.", format::unix_clock(at)));
                        // Others' findings may be visible now.
                        self.team_fetch_findings();
                    }
                    Err(e) => t.submit_error = Some(t.failed(e)),
                }
            }
        }
    }

    /// Draws the reports from the server like imported ones (they are not
    /// added to the history: the server keeps them).
    fn show_findings(&mut self, s: Submissions) {
        self.team.hidden = s.hidden_until_you_submit;
        let own = self.searches.record.as_ref().map(|r| r.id.clone());
        let mut shown = Vec::new();
        for sub in s.list {
            let Some(mut record) = Record::from_value(&sub.report) else {
                continue;
            };
            if Some(&record.id) == own.as_ref() {
                continue;
            }
            if record.searched_by.trim().is_empty() {
                record.searched_by = sub.user_name.clone();
            }
            shown.push(record.id.clone());
            self.add_review(record);
        }
        // Reports the server no longer has.
        let gone: Vec<String> = self
            .team
            .shared
            .iter()
            .filter(|id| !shown.contains(id))
            .cloned()
            .collect();
        self.reviews.retain(|r| !gone.contains(&r.record.id));
        self.team.shared = shown;
    }

    fn apply_url(&mut self) {
        let t = &mut self.team;
        match team::normalize_url(&t.url) {
            Ok(url) => {
                t.url_error = None;
                t.url.clone_from(&url);
                if url != t.config.url {
                    let gone = std::mem::take(&mut t.shared);
                    t.forget_session();
                    t.config.url = url;
                    t.save();
                    t.tested = None;
                    t.account_error = None;
                    t.account_note = None;
                    self.reviews.retain(|r| !gone.contains(&r.record.id));
                }
            }
            Err(e) => t.url_error = Some(e),
        }
    }

    fn sign_in(&mut self) {
        let t = &mut self.team;
        if t.signing_in || !t.configured() {
            return;
        }
        if t.email.trim().is_empty() || t.password.is_empty() {
            t.account_error = Some("Type your email and password.".into());
            return;
        }
        t.signing_in = true;
        t.account_error = None;
        t.account_note = None;
        let (url, email, password) = (t.config.url.clone(), t.email.clone(), t.password.clone());
        t.spawn(move || Event::SignedIn(team::login(&url, &email, &password)));
    }

    fn sign_out(&mut self) {
        let t = &mut self.team;
        let (url, token) = (t.config.url.clone(), t.config.token.clone());
        // Best effort: the token is forgotten here either way.
        t.spawn(move || {
            let _ = team::logout(&url, &token);
            Event::Done
        });
        let gone = std::mem::take(&mut t.shared);
        t.forget_session();
        t.account_error = None;
        t.account_note = None;
        self.reviews.retain(|r| !gone.contains(&r.record.id));
    }

    fn change_password(&mut self) {
        let t = &mut self.team;
        if t.changing {
            return;
        }
        if t.new_pw.is_empty() || t.current_pw.is_empty() {
            t.password_error = Some("Type the current and the new password.".into());
            return;
        }
        if t.new_pw != t.repeat_pw {
            t.password_error = Some("The new passwords don't match.".into());
            return;
        }
        t.changing = true;
        t.password_error = None;
        let (url, token, current, new) = (
            t.config.url.clone(),
            t.config.token.clone(),
            t.current_pw.clone(),
            t.new_pw.clone(),
        );
        t.spawn(move || Event::Password(team::change_password(&url, &token, &current, &new)));
    }

    // ------------------------------------------------------------------
    // Side panel

    /// Whether the panel has a "Team review" section for the open set.
    pub(super) fn team_has_section(&self) -> bool {
        let t = &self.team;
        self.set.is_some() && (t.matched.is_some() || t.looking_up || t.lookup_error.is_some())
    }

    /// The campaign badge, "Submit review" and what came of it.
    pub(super) fn team_section(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        section_heading(ui, "Team review", None);
        let t = &self.team;
        if t.looking_up {
            spinner_line(ui, "Asking the team server about this set…");
            return;
        }
        if let Some(e) = &t.lookup_error {
            small(ui, format!("Couldn't ask the team server: {e}"), p.danger);
            if ui.small_button("Try again").clicked() {
                self.team_lookup();
            }
            return;
        }
        let Some(m) = &t.matched else { return };
        Frame::new()
            .fill(p.accent_weak)
            .corner_radius(theme::RADIUS)
            .inner_margin(Margin::symmetric(S2 as i8, S1 as i8 + 2))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(Label::new(RichText::new(m.badge()).strong().color(p.accent)).wrap());
                let mut parts = Vec::new();
                if !m.team.name.is_empty() {
                    parts.push(m.team.name.clone());
                }
                if m.assigned {
                    parts.push("assigned to you".into());
                } else if m.role == "teacher" {
                    parts.push("you teach this team".into());
                }
                if let Some(s) = &m.status {
                    parts.push(capitalized(s));
                }
                if !parts.is_empty() {
                    small(ui, parts.join(" · "), p.text);
                }
            });
        // The server takes reviews from those assigned and from teachers.
        let allowed = m.assigned || m.role == "teacher";
        let can_submit = allowed
            && self.searches.record.is_some()
            && self.marking.is_none()
            && !t.submitting
            && t.ready();
        let w = ui.available_width();
        let button = ui
            .add_enabled(
                can_submit,
                primary(ui, "Submit review").min_size(vec2(w, theme::TOOL)),
            )
            .on_hover_text(
                "Send your search, with the objects you marked, to the team. You can \
                 submit again after changes.",
            )
            .on_disabled_hover_text(if !allowed {
                "This set is assigned to someone else"
            } else if self.marking.is_some() {
                "Finish marking the object first"
            } else {
                "Your search could not be started for this set"
            });
        let submit = button.clicked();
        let t = &self.team;
        if t.submitting {
            spinner_line(ui, "Submitting…");
        } else if let Some(e) = &t.submit_error {
            small(ui, e.clone(), p.danger);
        } else if let Some(n) = &t.submit_note {
            small(ui, n.clone(), p.muted);
        }
        let mut refresh = false;
        if t.fetching {
            spinner_line(ui, "Getting the team's findings…");
        } else if let Some(e) = &t.findings_error {
            small(
                ui,
                format!("Couldn't get the team's findings: {e}"),
                p.danger,
            );
            refresh = ui.small_button("Try again").clicked();
        } else {
            let text = if t.hidden {
                "The others' findings appear once you submit yours.".to_owned()
            } else {
                match t.shared.len() {
                    0 => "No one else has submitted this set yet.".to_owned(),
                    1 => "1 report from the team is shown below.".to_owned(),
                    n => format!("{n} reports from the team are shown below."),
                }
            };
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = S1;
                ui.label(RichText::new(text).small().color(p.muted));
                refresh = ui
                    .link(RichText::new("Refresh").small().color(p.accent))
                    .on_hover_text("Ask the server for new findings")
                    .clicked();
            });
        }
        if submit {
            self.team_submit();
        }
        if refresh {
            self.team_fetch_findings();
        }
    }

    // ------------------------------------------------------------------
    // The Team window

    pub(super) fn team_window(&mut self, ctx: &egui::Context) {
        if !self.team.window_open {
            return;
        }
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 2.0 * S5).min(WINDOW[0]),
            (screen.height() - 2.0 * S5).min(WINDOW[1]),
        )
        .max(vec2(320.0, 240.0));
        let style = ctx.global_style();
        let modal = egui::Modal::new(Id::new("team"))
            .frame(
                Frame::popup(&style)
                    .inner_margin(Margin::ZERO)
                    .corner_radius(theme::RADIUS + 2),
            )
            .backdrop_color(Color32::from_black_alpha(70))
            .show(ctx, |ui| self.team_body(ui, size));
        if modal.should_close() {
            self.team.window_open = false;
        }
    }

    fn team_body(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        let p = palette(ui);
        ui.set_width(size.x);
        ui.set_height(size.y);
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut refresh = false;
        let mut settings = false;
        Frame::new()
            .inner_margin(Margin::symmetric(S4 as i8, S3 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Team").heading().color(p.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::small_button(ui, Icon::Close, p.muted)
                            .on_hover_text("Close (Esc)")
                            .clicked()
                        {
                            self.team.window_open = false;
                        }
                        ui.add_space(S2);
                        let t = &self.team;
                        if t.ready() {
                            refresh = ui
                                .add_enabled(t.lists_pending == 0, Button::new("Refresh"))
                                .on_hover_text("Ask the server again")
                                .clicked();
                        }
                        settings = ui
                            .button("Account…")
                            .on_hover_text("Server and sign-in settings")
                            .clicked();
                    });
                });
            });
        hline(ui, p.border);
        egui::ScrollArea::vertical()
            .id_salt("team_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(S4 as i8, S3 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = S1;
                        self.team_lists(ui);
                    });
            });
        if refresh {
            self.team_refresh_lists();
        }
        if settings {
            self.open_settings(Category::Team);
        }
    }

    fn team_lists(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        if !self.team.config.signed_in() {
            muted_wrapped(
                ui,
                "Sign in to your team server (Account…) to see your assignments and \
                 the team's announcements.",
            );
            return;
        }
        if self.team.must_change {
            self.password_form(ui, true);
            return;
        }
        let t = &self.team;
        let who = match (&t.me, t.config.name.as_str()) {
            (Some(me), _) if !me.user.name.is_empty() => me.user.name.clone(),
            (_, "") => t.config.email.clone(),
            (_, name) => name.to_owned(),
        };
        let mut line = format!("Signed in as {who}");
        if let Some(me) = &t.me
            && !me.teams.is_empty()
        {
            let teams: Vec<String> = me
                .teams
                .iter()
                .map(|team| match team.role.as_str() {
                    "" => team.name.clone(),
                    role => format!("{} ({role})", team.name),
                })
                .collect();
            line += &format!(" · {}", teams.join(", "));
        }
        small(ui, line, p.muted);
        if t.lists_pending > 0 {
            spinner_line(ui, "Loading…");
        }
        if let Some(e) = &t.lists_error {
            small(ui, e.clone(), p.danger);
        }

        ui.add_space(S3);
        ui.label(RichText::new("My assignments").strong().color(p.text));
        match &t.assignments {
            None => {}
            Some(list) if list.is_empty() => {
                muted_wrapped(ui, "Nothing assigned to you yet.");
            }
            Some(list) => {
                let open = t.matched.as_ref().map(|m| m.set_id.as_str());
                for (i, a) in list.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(S1);
                    }
                    assignment_row(ui, a, open == Some(a.set_id.as_str()));
                }
            }
        }

        ui.add_space(S4);
        ui.label(RichText::new("Announcements").strong().color(p.text));
        match &t.announcements {
            None => {}
            Some(list) if list.is_empty() => muted_wrapped(ui, "No announcements."),
            Some(list) => {
                for (i, a) in list.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(S2);
                        hline(ui, p.border);
                        ui.add_space(S2);
                    }
                    announcement(ui, a);
                }
            }
        }
    }

    /// New password (`forced`: the server asks for it before anything else).
    fn password_form(&mut self, ui: &mut egui::Ui, forced: bool) {
        let p = palette(ui);
        if forced {
            ui.label(
                RichText::new("Choose a new password")
                    .strong()
                    .color(p.text),
            );
            muted_wrapped(
                ui,
                "Your password was reset. Type the temporary one you were given, then \
                 a new password of your own.",
            );
        }
        let t = &mut self.team;
        let w = FIELD.min(ui.available_width());
        let current_hint = if forced {
            "Temporary password"
        } else {
            "Current password"
        };
        ui.add(
            TextEdit::singleline(&mut t.current_pw)
                .password(true)
                .hint_text(current_hint)
                .desired_width(w),
        );
        ui.add(
            TextEdit::singleline(&mut t.new_pw)
                .password(true)
                .hint_text("New password")
                .desired_width(w),
        );
        let repeat = ui.add(
            TextEdit::singleline(&mut t.repeat_pw)
                .password(true)
                .hint_text("New password again")
                .desired_width(w),
        );
        let enter = repeat.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let mut cancel = false;
        let go = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S2;
                let go = ui
                    .add_enabled(!t.changing, primary(ui, "Set new password"))
                    .clicked();
                if !forced {
                    cancel = ui.button("Cancel").clicked();
                }
                go
            })
            .inner;
        if t.changing {
            spinner_line(ui, "Changing the password…");
        } else if let Some(e) = &t.password_error {
            small(ui, e.clone(), p.danger);
        }
        if cancel {
            t.password_open = false;
            t.current_pw.clear();
            t.new_pw.clear();
            t.repeat_pw.clear();
            t.password_error = None;
        }
        if go || enter {
            self.change_password();
        }
    }
}

fn assignment_row(ui: &mut egui::Ui, a: &Assignment, open: bool) {
    let p = palette(ui);
    Frame::new()
        .fill(p.inset)
        .corner_radius(theme::RADIUS)
        .stroke(if open {
            Stroke::new(1.0, p.accent)
        } else {
            Stroke::NONE
        })
        .inner_margin(Margin::symmetric(S2 as i8, S1 as i8 + 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(&a.code).strong().color(p.text));
                if open {
                    ui.label(RichText::new("open now").small().color(p.accent));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let status = a
                        .status
                        .as_deref()
                        .map_or("Not started".into(), capitalized);
                    ui.label(RichText::new(status).small().color(p.muted));
                });
            });
            let mut parts = vec![a.campaign.name.clone(), a.team.name.clone()];
            match (a.campaign.starts_on.as_str(), a.campaign.ends_on.as_str()) {
                ("", "") => {}
                (from, "") => parts.push(format!("from {from}")),
                ("", to) => parts.push(format!("until {to}")),
                (from, to) => parts.push(format!("{from} – {to}")),
            }
            parts.retain(|s| !s.trim().is_empty());
            small(ui, parts.join(" · "), p.muted);
        });
}

fn announcement(ui: &mut egui::Ui, a: &Announcement) {
    let p = palette(ui);
    ui.add(Label::new(RichText::new(&a.title).color(p.text)).wrap());
    let mut parts = vec![a.team_name.clone(), a.author.clone()];
    if a.created_at > 0 {
        parts.push(format::unix_utc(a.created_at));
    }
    parts.retain(|s| !s.trim().is_empty());
    small(ui, parts.join(" · "), p.muted);
    if !a.body.trim().is_empty() {
        ui.add(Label::new(RichText::new(a.body.trim()).color(p.text)).wrap());
    }
}

// ---------------------------------------------------------------------------
// Settings rows (see the Team page in `crate::settings`)

pub fn server_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let t = &mut app.team;
    let w = FIELD.min(ui.available_width());
    let edit = ui.add(
        TextEdit::singleline(&mut t.url)
            .hint_text("https://team.example.org")
            .desired_width(w),
    );
    let apply = edit.lost_focus();
    let (mut test, mut offline) = (false, false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        test = ui
            .add_enabled(
                !t.testing && !t.url.trim().is_empty(),
                Button::new("Test connection"),
            )
            .on_hover_text("Ask the server whether it is an OpenCeres Team server")
            .clicked();
        if t.configured()
            && ui
                .button("Stay offline")
                .on_hover_text("Forget the address and sign out")
                .clicked()
        {
            t.url.clear();
            offline = true;
        }
    });
    if apply || test || offline {
        app.apply_url();
    }
    let t = &mut app.team;
    if let Some(e) = &t.url_error {
        small(ui, e.clone(), p.danger);
    } else if t.testing {
        spinner_line(ui, "Testing…");
    } else if let Some(r) = &t.tested {
        match r {
            Ok(m) => small(ui, m.clone(), p.muted),
            Err(e) => small(ui, e.clone(), p.danger),
        }
    }
    if team::is_insecure(&t.config.url) {
        small(
            ui,
            "This address isn't encrypted (http://): your password could be read on \
             the way. Use https:// if the server has it.",
            p.danger,
        );
    }
    if test && t.url_error.is_none() && t.configured() {
        t.testing = true;
        t.tested = None;
        let url = t.config.url.clone();
        t.spawn(move || Event::Tested(team::info(&url)));
    }
}

pub fn account_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    if !app.team.configured() {
        muted_wrapped(ui, "Type the server address above first.");
        return;
    }
    if app.team.config.signed_in() {
        let t = &app.team;
        let who = match (t.config.name.as_str(), t.config.email.as_str()) {
            ("", email) => email.to_owned(),
            (name, "") => name.to_owned(),
            (name, email) => format!("{name} ({email})"),
        };
        ui.label(RichText::new(format!("Signed in as {who}")).color(p.text));
        if let Some(n) = &t.account_note {
            small(ui, n.clone(), p.muted);
        }
        if t.must_change || t.password_open {
            let forced = t.must_change;
            app.password_form(ui, forced);
        }
        let t = &app.team;
        let (mut out, mut change, mut window) = (false, false, false);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            window = ui
                .add_enabled(t.ready(), Button::new("Open Team window"))
                .on_hover_text("Your assignments and the team's announcements")
                .clicked();
            if !t.must_change && !t.password_open {
                change = ui.button("Change password").clicked();
            }
            out = ui.button("Sign out").clicked();
        });
        if change {
            app.team.password_open = true;
            app.team.password_error = None;
        }
        if window {
            app.settings_open = false;
            app.toggle_team();
        }
        if out {
            app.sign_out();
        }
        return;
    }
    let t = &mut app.team;
    let w = FIELD.min(ui.available_width());
    ui.add(
        TextEdit::singleline(&mut t.email)
            .hint_text("Email")
            .desired_width(w),
    );
    let pw = ui.add(
        TextEdit::singleline(&mut t.password)
            .password(true)
            .hint_text("Password")
            .desired_width(w),
    );
    let enter = pw.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    let go = ui
        .add_enabled(!t.signing_in, primary(ui, "Sign in"))
        .clicked();
    if t.signing_in {
        spinner_line(ui, "Signing in…");
    } else if let Some(e) = &t.account_error {
        small(ui, e.clone(), p.danger);
    } else {
        small(
            ui,
            "Your teacher gives you an account or an invite code for the server.",
            p.muted,
        );
    }
    if go || enter {
        app.sign_in();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_well() {
        assert_eq!(capitalized("in_progress"), "In progress");
        assert_eq!(capitalized(""), "");
        assert_eq!(capitalized("ödev"), "Ödev");
    }
}
