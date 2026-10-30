//! The search history in the window: the "Your search" section of the side
//! panel, saving as the student works, and the History window (search,
//! filters, status updates, export and import). Storage is in
//! [`crate::history`].

use eframe::egui::{
    self, Align, Button, Color32, CursorIcon, FontId, Frame, Id, Label, Layout, Margin, Response,
    RichText, Sense, Stroke, TextEdit, pos2, text::LayoutJob, vec2,
};

use super::{App, KnownState, hline, muted_wrapped, segmented, settings_ui};
use crate::format;
use crate::history::{self, Record, Status, box_label, same_name};
use crate::icons::{self, Icon};
use crate::prefs::GRID_SIZES;
use crate::theme::{self, S1, S2, S3, S4, S5, palette};
use crate::update;
use openceres_core::known::KnownObject;

const WINDOW: [f32; 2] = [760.0, 560.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Mine,
    Reported,
    Finds,
}

impl Filter {
    const ALL: [Filter; 4] = [Filter::All, Filter::Mine, Filter::Reported, Filter::Finds];

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Mine => "Mine",
            Filter::Reported => "Reported",
            Filter::Finds => "Finds",
        }
    }
}

/// Something a history row asks for, done after the list is drawn.
enum Action {
    Open(Record),
    Delete(Record),
    Saved(Record),
}

impl App {
    pub(super) fn modal_open(&self) -> bool {
        self.settings_open || self.searches.window_open
    }

    pub(super) fn toggle_history(&mut self) {
        let h = &mut self.searches;
        h.window_open = !h.window_open;
        if h.window_open {
            h.list = None; // read the folder again: others may have added to it
            h.note = None;
            h.confirm_delete = None;
            self.settings_open = false;
            self.help_open = false;
        }
    }

    /// A set has opened: continue this student's earlier search of it (or
    /// the one chosen in the History window), else start a new one, and
    /// note who else searched it.
    pub(super) fn start_search(&mut self) {
        let resume_id = self.searches.resume_id.take();
        let h = &mut self.searches;
        h.record = None;
        h.saved = None;
        h.resumed = None;
        h.others.clear();
        h.list = None;
        let Some(set) = &self.set else { return };
        if !h.enabled {
            return;
        }
        let name = if set.name.is_empty() {
            "Image set".to_owned()
        } else {
            set.name.clone()
        };
        let first = set.frames.first();
        let observed = first.and_then(|f| f.date_obs.clone());
        let observatory = first.and_then(|f| f.observatory).map(str::to_owned);
        let paths = set.paths.clone();
        let all = match h.config.dir().map(|d| history::load_all(&d)) {
            Some(Ok((all, _))) => all,
            Some(Err(e)) => {
                h.error = Some(e);
                Vec::new()
            }
            None => Vec::new(),
        };
        let me = h.config.name.clone();
        let same: Vec<&Record> = all
            .iter()
            .filter(|r| r.is_set(&name, observed.as_deref()))
            .collect();
        let previous = resume_id
            .and_then(|id| all.iter().find(|r| r.id == id))
            .or_else(|| same.iter().copied().find(|r| r.is_by(&me)));
        let owner = previous.map_or(me.as_str(), |r| r.searched_by.as_str());
        for r in &same {
            let who = r.searched_by.trim();
            if !who.is_empty()
                && !same_name(who, owner)
                && !h.others.iter().any(|o| same_name(o, who))
            {
                h.others.push(who.to_owned());
            }
        }
        let record = match previous {
            Some(prev) => {
                h.saved = Some(prev.clone());
                h.resumed = Some(prev.updated);
                let mut r = prev.clone();
                r.files = paths; // where the images are now
                r.observed = r.observed.or(observed);
                r.observatory = r.observatory.or(observatory);
                r
            }
            None => {
                let mut r = Record::new(&name, &me, self.grid_size);
                r.observed = observed;
                r.observatory = observatory;
                r.files = paths;
                r
            }
        };
        if h.resumed.is_some() {
            if GRID_SIZES.contains(&record.grid) {
                self.grid_size = record.grid;
            }
            self.marks.clone_from(&record.boxes);
            if !self.marks.is_empty() {
                self.set_grid(true);
            }
        }
        self.searches.record = Some(record);
    }

    /// Forgets the open set's earlier search and starts a new one.
    fn start_new_search(&mut self) {
        let h = &mut self.searches;
        let Some(old) = &h.record else { return };
        let mut r = Record::new(&old.set, &h.config.name, self.grid_size);
        r.observed.clone_from(&old.observed);
        r.observatory.clone_from(&old.observatory);
        r.files.clone_from(&old.files);
        h.record = Some(r);
        h.saved = None;
        h.resumed = None;
        self.marks.clear();
        self.copied = false;
    }

    /// Copying the report counts as reporting it.
    pub(super) fn mark_reported(&mut self) {
        if let Some(r) = &mut self.searches.record
            && matches!(r.status, Status::Searching | Status::NothingFound)
        {
            r.status = Status::Reported;
        }
    }

    /// Brings the open search up to date with the window and writes it when
    /// it changed. Runs every frame; writes only on change.
    pub(super) fn autosave(&mut self) {
        let h = &mut self.searches;
        let Some(r) = &mut h.record else { return };
        r.boxes.clone_from(&self.marks);
        r.grid = self.grid_size;
        if let Some(KnownState::Done(k)) = &self.known {
            let names: Vec<String> = k.objects.iter().map(|o| known_name(&o.object)).collect();
            if r.known != names {
                r.known = names;
            }
        }
        if !h.enabled {
            return;
        }
        let changed = match &h.saved {
            Some(s) => {
                let updated = r.updated;
                r.updated = s.updated;
                let same = *r == *s;
                r.updated = updated;
                !same
            }
            None => r.has_content(),
        };
        if !changed {
            return;
        }
        r.updated = format::unix_now().max(r.updated);
        let result = h
            .config
            .dir()
            .ok_or_else(|| "There is no folder for the search history.".to_owned())
            .and_then(|d| history::save(&d, r));
        // Remembered even when saving failed, so it is not retried every
        // frame; the next change tries again.
        h.saved = Some(r.clone());
        h.error = result.err();
        if let Some(list) = &mut h.list {
            match list.iter_mut().find(|x| x.id == r.id) {
                Some(x) => x.clone_from(r),
                None => list.insert(0, r.clone()),
            }
        }
    }

    /// "Your search" in the side panel: who searched, how it went, notes.
    pub(super) fn search_section(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let h = &mut self.searches;
        let Some(r) = &mut h.record else { return };
        ui.horizontal(|ui| {
            ui.label(RichText::new("Your search").strong().color(p.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(e) = &h.error {
                    ui.label(RichText::new("Not saved").small().color(p.danger))
                        .on_hover_text(e);
                } else if h.saved.is_some() {
                    ui.label(RichText::new("Saved").small().color(p.muted))
                        .on_hover_text("Saved in the history as you work (H to see it)");
                }
            });
        });
        let w = ui.available_width();
        let name = ui
            .add(
                TextEdit::singleline(&mut r.searched_by)
                    .hint_text("Your name")
                    .desired_width(w),
            )
            .on_hover_text("Searched by. Also filled in for your next searches.");
        if name.changed() {
            h.config.name = r.searched_by.trim().to_owned();
            if self.capture.is_none() {
                h.config.save();
            }
        }
        status_combo(ui, Id::new("search_status"), &mut r.status, w);
        if r.status.has_designation() {
            ui.add(
                TextEdit::singleline(&mut r.designation)
                    .hint_text("Designation, e.g. 2026 UX12")
                    .desired_width(w),
            );
        }
        ui.add(
            TextEdit::multiline(&mut r.notes)
                .hint_text("Notes")
                .desired_rows(2)
                .desired_width(w),
        );
        let mut fresh = false;
        if let Some(t) = h.resumed {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = S1;
                ui.label(
                    RichText::new(format!("Continued from {} ·", day(t)))
                        .small()
                        .color(p.muted),
                );
                fresh = ui
                    .link(RichText::new("Start new").small().color(p.accent))
                    .on_hover_text("Keep the earlier search in the history and start again")
                    .clicked();
            });
        }
        if !h.others.is_empty() {
            ui.add(
                Label::new(
                    RichText::new(format!("Also searched by {}", h.others.join(", ")))
                        .small()
                        .color(p.muted),
                )
                .wrap(),
            );
        }
        if fresh {
            self.start_new_search();
        }
    }

    // ----------------------------------------------------------------------
    // History window

    pub(super) fn reload_history(&mut self) {
        let h = &mut self.searches;
        match h.config.dir().map(|d| history::load_all(&d)) {
            Some(Ok((list, unreadable))) => {
                h.list = Some(list);
                h.unreadable = unreadable;
            }
            Some(Err(e)) => {
                h.list = Some(Vec::new());
                h.error = Some(e);
            }
            None => h.list = Some(Vec::new()),
        }
    }

    pub(super) fn history_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if !self.searches.window_open {
            return;
        }
        if self.searches.list.is_none() {
            self.reload_history();
        }
        let screen = ctx.content_rect();
        let size = vec2(
            (screen.width() - 2.0 * S5).min(WINDOW[0]),
            (screen.height() - 2.0 * S5).min(WINDOW[1]),
        )
        .max(vec2(340.0, 260.0));
        let style = ctx.global_style();
        let modal = egui::Modal::new(Id::new("history"))
            .frame(
                Frame::popup(&style)
                    .inner_margin(Margin::ZERO)
                    .corner_radius(theme::RADIUS + 2),
            )
            .backdrop_color(Color32::from_black_alpha(70))
            .show(ctx, |ui| self.history_body(ui, size, frame));
        if modal.should_close() {
            self.searches.window_open = false;
        }
    }

    fn history_body(&mut self, ui: &mut egui::Ui, size: egui::Vec2, frame: &eframe::Frame) {
        let p = palette(ui);
        ui.set_width(size.x);
        ui.set_height(size.y);
        ui.spacing_mut().item_spacing.y = 0.0;

        // Header: title, search, close.
        Frame::new()
            .inner_margin(Margin::symmetric(S4 as i8, S3 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("History").heading().color(p.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icons::small_button(ui, Icon::Close, p.muted)
                            .on_hover_text("Close (Esc)")
                            .clicked()
                        {
                            self.searches.window_open = false;
                        }
                        ui.add_space(S2);
                        let w = (ui.available_width() - 120.0).clamp(140.0, 260.0);
                        settings_ui::search_box(
                            ui,
                            &mut self.searches.search,
                            "Search sets, names, notes",
                            "history_search",
                            w,
                        );
                    });
                });
            });
        hline(ui, p.border);

        // Filters, import and export.
        let shown = self.shown_records();
        Frame::new()
            .inner_margin(Margin::symmetric(S4 as i8, S2 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let options: Vec<(Filter, String)> = Filter::ALL
                        .iter()
                        .map(|&f| (f, f.label().to_owned()))
                        .collect();
                    let mut filter = self.searches.filter;
                    let w = 300.0f32.min(ui.available_width() - 190.0).max(200.0);
                    ui.allocate_ui_with_layout(
                        vec2(w, theme::TOOL),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            if segmented(ui, &mut filter, &options, w) {
                                self.searches.filter = filter;
                                self.searches.expanded = None;
                            }
                        },
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = S2;
                        let label = format!("Export {}…", shown.len());
                        ui.add_enabled_ui(!shown.is_empty(), |ui| {
                            ui.menu_button(label, |ui| {
                                if ui.button("For importing (JSON)").clicked() {
                                    self.export(frame, &shown, false);
                                    ui.close();
                                }
                                if ui.button("As a spreadsheet (CSV)").clicked() {
                                    self.export(frame, &shown, true);
                                    ui.close();
                                }
                            })
                            .response
                            .on_hover_text("Save the searches shown below to a file");
                        });
                        if ui
                            .button("Import…")
                            .on_hover_text("Add searches from an exported file")
                            .clicked()
                        {
                            self.import(frame);
                        }
                    });
                });
            });

        // Summary and messages.
        Frame::new()
            .inner_margin(Margin {
                left: S4 as i8,
                right: S4 as i8,
                top: 0,
                bottom: S2 as i8,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = S1;
                let h = &self.searches;
                let all = h.list.as_deref().unwrap_or_default();
                let reported = all.iter().filter(|r| r.status.is_reported()).count();
                let finds = all.iter().filter(|r| r.status.is_find()).count();
                let plural = |n: usize, one: &str, many: &str| {
                    format!("{n} {}", if n == 1 { one } else { many })
                };
                let mut summary = format!(
                    "{} · {} reported · {} accepted by the MPC",
                    plural(all.len(), "search", "searches"),
                    reported,
                    finds
                );
                if h.unreadable > 0 {
                    summary += &format!(" · {} file(s) couldn't be read", h.unreadable);
                }
                ui.label(RichText::new(summary).small().color(p.muted));
                if let Some(e) = &h.error {
                    ui.add(Label::new(RichText::new(e).small().color(p.danger)).wrap());
                }
                if let Some(n) = &h.note {
                    ui.add(Label::new(RichText::new(n).small().color(p.accent)).wrap());
                }
            });
        hline(ui, p.border);

        // The list.
        let mut action = None;
        egui::ScrollArea::vertical()
            .id_salt("history_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                Frame::new()
                    .inner_margin(Margin::symmetric(S3 as i8, S2 as i8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if shown.is_empty() {
                            ui.add_space(S4);
                            muted_wrapped(ui, &self.empty_message());
                        }
                        for r in &shown {
                            if let Some(a) = self.history_row(ui, r) {
                                action = Some(a);
                            }
                        }
                    });
            });
        match action {
            Some(Action::Open(r)) => {
                let existing: Vec<String> = r
                    .files
                    .iter()
                    .filter(|f| std::path::Path::new(f).exists())
                    .cloned()
                    .collect();
                self.searches.resume_id = Some(r.id);
                self.searches.window_open = false;
                self.open(ui.ctx(), existing);
            }
            Some(Action::Delete(r)) => self.delete_search(&r),
            Some(Action::Saved(r)) => {
                let h = &mut self.searches;
                if let Some(list) = &mut h.list
                    && let Some(x) = list.iter_mut().find(|x| x.id == r.id)
                {
                    x.clone_from(&r);
                }
                // The open search changed here: take the new status and notes.
                if let Some(cur) = &mut h.record
                    && cur.id == r.id
                {
                    cur.status = r.status;
                    cur.designation.clone_from(&r.designation);
                    cur.notes.clone_from(&r.notes);
                    cur.updated = r.updated;
                    h.saved = Some(cur.clone());
                }
            }
            None => {}
        }
    }

    fn shown_records(&self) -> Vec<Record> {
        let h = &self.searches;
        let query = h.search.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        h.list
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|r| match h.filter {
                Filter::All => true,
                Filter::Mine => r.is_by(&h.config.name),
                Filter::Reported => r.status.is_reported(),
                Filter::Finds => r.status.is_find(),
            })
            .filter(|r| {
                let text = format!(
                    "{} {} {} {} {} {}",
                    r.set,
                    r.searched_by,
                    r.status.label(),
                    r.designation,
                    r.notes,
                    r.known.join(" ")
                )
                .to_lowercase();
                words.iter().all(|w| text.contains(w))
            })
            .cloned()
            .collect()
    }

    fn empty_message(&self) -> String {
        let h = &self.searches;
        if !h.search.trim().is_empty() {
            format!("No searches match “{}”.", h.search.trim())
        } else if h.list.as_ref().is_some_and(|l| !l.is_empty()) {
            "No searches here yet.".into()
        } else {
            "Your searches appear here once you mark boxes, take notes or copy a \
             report. Each one is saved on its own, so nothing is lost."
                .into()
        }
    }

    /// One search: a summary line that opens into details and edits.
    fn history_row(&mut self, ui: &mut egui::Ui, r: &Record) -> Option<Action> {
        let p = palette(ui);
        let expanded = self.searches.expanded.as_deref() == Some(r.id.as_str());
        let current = self.searches.record.as_ref().is_some_and(|c| c.id == r.id);
        if row_header(ui, r, expanded, current).clicked() {
            self.searches.expanded = if expanded { None } else { Some(r.id.clone()) };
            self.searches.confirm_delete = None;
        }
        if !expanded {
            return None;
        }
        let mut action = None;
        let mut edited = r.clone();
        Frame::new()
            .fill(p.inset)
            .corner_radius(theme::RADIUS)
            .inner_margin(Margin::same(S3 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = S2;
                if !r.boxes.is_empty() {
                    ui.add(
                        Label::new(RichText::new(r.report()).monospace().color(p.text))
                            .wrap()
                            .selectable(true),
                    );
                }
                let w = ui.available_width().min(360.0);
                egui::Grid::new(("history_edit", &r.id))
                    .num_columns(2)
                    .spacing(vec2(S3, S2))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Status").color(p.muted));
                        status_combo(
                            ui,
                            Id::new(("history_status", &r.id)),
                            &mut edited.status,
                            w,
                        );
                        ui.end_row();
                        if edited.status.has_designation() {
                            ui.label(RichText::new("Designation").color(p.muted));
                            ui.add(
                                TextEdit::singleline(&mut edited.designation)
                                    .hint_text("e.g. 2026 UX12")
                                    .desired_width(w),
                            );
                            ui.end_row();
                        }
                        ui.label(RichText::new("Notes").color(p.muted));
                        ui.add(
                            TextEdit::multiline(&mut edited.notes)
                                .hint_text("Notes")
                                .desired_rows(2)
                                .desired_width(w),
                        );
                        ui.end_row();
                    });
                let mut facts = Vec::new();
                if let Some(t) = &r.observed {
                    let at = r
                        .observatory
                        .as_deref()
                        .map_or_else(String::new, |o| format!(" at {o}"));
                    facts.push(format!("Observed {}{at}", format::obs_time(Some(t), None)));
                }
                facts.push(format!(
                    "Searched {} · last changed {}",
                    format::unix_utc(r.created),
                    format::unix_utc(r.updated)
                ));
                if !r.known.is_empty() {
                    facts.push(format!(
                        "Known objects in the field: {}",
                        r.known.join(", ")
                    ));
                }
                for f in facts {
                    ui.add(Label::new(RichText::new(f).small().color(p.muted)).wrap());
                }
                let can_open = r.files.iter().any(|f| std::path::Path::new(f).exists());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = S2;
                    let open = ui
                        .add_enabled(!current && can_open, Button::new("Open images"))
                        .on_hover_text("Open this set again, with its marked boxes")
                        .on_disabled_hover_text(if current {
                            "This set is open now"
                        } else {
                            "The image files are no longer where they were opened from"
                        });
                    if open.clicked() {
                        action = Some(Action::Open(r.clone()));
                    }
                    if ui
                        .add_enabled(!r.boxes.is_empty(), Button::new("Copy report"))
                        .clicked()
                    {
                        ui.ctx().copy_text(r.report());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.searches.confirm_delete.as_deref() == Some(r.id.as_str()) {
                            if ui.button("Cancel").clicked() {
                                self.searches.confirm_delete = None;
                            }
                            let delete = Button::new(RichText::new("Delete").color(p.accent_text))
                                .fill(p.danger)
                                .stroke(Stroke::NONE);
                            if ui.add(delete).clicked() {
                                action = Some(Action::Delete(r.clone()));
                            }
                        } else if ui
                            .button("Delete…")
                            .on_hover_text("Remove this search from the history")
                            .clicked()
                        {
                            self.searches.confirm_delete = Some(r.id.clone());
                        }
                    });
                });
            });
        ui.add_space(S1);
        if action.is_none() && edited != *r {
            edited.updated = format::unix_now().max(r.updated + 1);
            let result = self
                .searches
                .config
                .dir()
                .ok_or_else(|| "There is no folder for the search history.".to_owned())
                .and_then(|d| history::save(&d, &edited));
            self.searches.error = result.err();
            action = Some(Action::Saved(edited));
        }
        action
    }

    fn delete_search(&mut self, r: &Record) {
        let h = &mut self.searches;
        h.confirm_delete = None;
        h.expanded = None;
        let result = h.config.dir().map_or(Ok(()), |d| history::delete(&d, r));
        if let Err(e) = result {
            h.error = Some(e);
            return;
        }
        if let Some(list) = &mut h.list {
            list.retain(|x| x.id != r.id);
        }
        if h.record.as_ref().is_some_and(|c| c.id == r.id) {
            // The open search is gone: start the set afresh, without marks,
            // so it is not saved straight back.
            self.start_new_search();
        }
    }

    fn export(&mut self, frame: &eframe::Frame, records: &[Record], csv: bool) {
        let (ext, kind) = if csv {
            ("csv", "Spreadsheet")
        } else {
            ("json", "OpenCeres history")
        };
        let name = format!("OpenCeres history {}.{ext}", day(format::unix_now()));
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export searches")
            .set_file_name(name)
            .add_filter(kind, &[ext])
            .set_parent(frame)
            .save_file()
        else {
            return;
        };
        let text = if csv {
            // A byte order mark tells Excel the file is UTF-8 (names with ş, ğ…).
            format!("\u{feff}{}", history::export_csv(records))
        } else {
            history::export_json(records)
        };
        let h = &mut self.searches;
        match std::fs::write(&path, text) {
            Ok(()) => {
                let n = records.len();
                h.note = Some(format!(
                    "Exported {n} search{} to {}",
                    if n == 1 { "" } else { "es" },
                    path.display()
                ));
                h.error = None;
                update::reveal(&path);
            }
            Err(e) => h.error = Some(format!("Couldn't export: {e}")),
        }
    }

    fn import(&mut self, frame: &eframe::Frame) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import searches")
            .add_filter("OpenCeres history", &["json"])
            .set_parent(frame)
            .pick_file()
        else {
            return;
        };
        let h = &mut self.searches;
        let Some(dir) = h.config.dir() else {
            h.error = Some("There is no folder for the search history.".into());
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|e| format!("Couldn't read {}: {e}", path.display()))
            .and_then(|text| history::import(&dir, &text));
        match result {
            Ok(got) => {
                let mut parts = Vec::new();
                if got.added > 0 {
                    parts.push(format!("{} new", got.added));
                }
                if got.updated > 0 {
                    parts.push(format!("{} updated", got.updated));
                }
                if got.unchanged > 0 {
                    parts.push(format!("{} already here", got.unchanged));
                }
                h.note = Some(format!("Imported: {}.", parts.join(", ")));
                h.error = None;
                h.list = None;
            }
            Err(e) => {
                h.error = Some(e);
                h.note = None;
            }
        }
    }
}

/// `133105 2003 OG10`, or the name alone for unnumbered objects.
fn known_name(o: &KnownObject) -> String {
    match &o.number {
        Some(n) => format!("{n} {}", o.name),
        None => o.name.clone(),
    }
}

/// `29 Oct 2026` for a Unix time.
fn day(secs: u64) -> String {
    format::unix_utc(secs)
        .splitn(4, ' ')
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

fn status_combo(ui: &mut egui::Ui, id: Id, status: &mut Status, width: f32) {
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(status.label())
        .show_ui(ui, |ui| {
            for s in Status::ALL {
                ui.selectable_value(status, s, s.label());
            }
        })
        .response
        .on_hover_text("Where this search stands. Copying the report marks it Reported.");
}

/// The always-visible part of a history row.
fn row_header(ui: &mut egui::Ui, r: &Record, expanded: bool, current: bool) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let painter = ui.painter();
    if expanded {
        painter.rect_filled(rect, theme::RADIUS, p.accent_weak);
    } else if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, p.hover);
    }
    let inner = rect.shrink2(vec2(S2, S1 + 1.0));
    let format = |size: f32, color: Color32| egui::TextFormat {
        font_id: FontId::proportional(size),
        color,
        ..Default::default()
    };
    // Status pill on the right.
    let (pill_text, pill_color) = match r.status {
        s if s.is_find() && !r.designation.trim().is_empty() => (
            format!("{} · {}", s.label(), r.designation.trim()),
            p.accent,
        ),
        s if s.is_find() => (s.label().to_owned(), p.accent),
        s @ (Status::Reported | Status::Searching) => (s.label().to_owned(), p.text),
        s => (s.label().to_owned(), p.muted),
    };
    let pill = painter.layout_no_wrap(pill_text, FontId::proportional(11.5), pill_color);
    let pill_size = pill.size() + vec2(2.0 * S2, 6.0);
    let pill_rect = egui::Rect::from_min_size(
        pos2(
            inner.max.x - pill_size.x,
            inner.center().y - pill_size.y / 2.0,
        ),
        pill_size,
    );
    let fill = if r.status.is_find() {
        p.accent_weak
    } else {
        p.inset
    };
    painter.rect_filled(pill_rect, 9.0, fill);
    painter.galley(pill_rect.min + vec2(S2, 3.0), pill, pill_color);

    let text_w = (pill_rect.min.x - inner.min.x - S3).max(40.0);
    let one_line = |job: &mut LayoutJob| {
        job.wrap = egui::text::TextWrapping {
            max_width: text_w,
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
    };
    let mut title = LayoutJob::default();
    title.append(&r.set, 0.0, format(13.0, p.text));
    if let Some(d) = r.observed.as_deref().and_then(|o| o.get(..10)) {
        title.append(d, S2, format(12.0, p.muted));
    }
    if current {
        title.append("open now", S2, format(11.5, p.accent));
    }
    one_line(&mut title);
    let mut boxes = r.boxes.clone();
    boxes.sort_unstable();
    let boxes: Vec<String> = boxes.into_iter().map(box_label).collect();
    let who = if r.searched_by.trim().is_empty() {
        "No name"
    } else {
        r.searched_by.trim()
    };
    let mut parts = vec![who.to_owned(), day(r.updated)];
    if !boxes.is_empty() {
        parts.push(boxes.join(" "));
    }
    let mut sub = LayoutJob::default();
    sub.append(&parts.join(" · "), 0.0, format(11.5, p.muted));
    one_line(&mut sub);
    let title = painter.layout_job(title);
    let sub = painter.layout_job(sub);
    painter.galley(inner.min, title, p.text);
    painter.galley(pos2(inner.min.x, inner.max.y - sub.size().y), sub, p.muted);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

// ---------------------------------------------------------------------------
// Settings rows (see the History page in `crate::settings`)

pub fn name_setting(app: &mut App, ui: &mut egui::Ui) {
    let edit = ui.add(
        TextEdit::singleline(&mut app.searches.config.name)
            .hint_text("Your name")
            .desired_width(260.0),
    );
    if edit.changed() && app.capture.is_none() {
        app.searches.config.save();
    }
}

pub fn folder_setting(app: &mut App, ui: &mut egui::Ui) {
    let p = palette(ui);
    let h = &mut app.searches;
    let Some(dir) = h.config.dir() else {
        ui.label(RichText::new("No folder could be found on this computer.").color(p.danger));
        return;
    };
    ui.add(
        Label::new(
            RichText::new(dir.display().to_string())
                .monospace()
                .size(11.5)
                .color(p.text),
        )
        .wrap()
        .selectable(true),
    );
    if std::env::var_os("OPENCERES_HISTORY_DIR").is_some() {
        ui.label(
            RichText::new("Set by OPENCERES_HISTORY_DIR")
                .small()
                .color(p.muted),
        );
    }
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        if ui
            .button("Choose folder…")
            .on_hover_text("For example a shared OneDrive, Google Drive or network folder")
            .clicked()
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Folder for the search history")
                .pick_folder()
        {
            h.config.folder = Some(folder);
            changed = true;
        }
        if h.config.folder.is_some()
            && ui
                .button("Use the default")
                .on_hover_text("Keep the history in the data folder on this computer")
                .clicked()
        {
            h.config.folder = None;
            changed = true;
        }
        let label = if cfg!(target_os = "macos") {
            "Show in Finder"
        } else if cfg!(windows) {
            "Show in Explorer"
        } else {
            "Open folder"
        };
        if ui.button(label).clicked() {
            let _ = std::fs::create_dir_all(&dir);
            update::open_url(&dir.to_string_lossy());
        }
    });
    if changed {
        if app.capture.is_none() {
            app.searches.config.save();
        }
        app.searches.list = None;
    }
}

pub fn open_history(app: &mut App, ui: &mut egui::Ui) {
    if ui.button("Open history").clicked() {
        app.settings_open = false;
        app.searches.window_open = false;
        app.toggle_history();
    }
}
