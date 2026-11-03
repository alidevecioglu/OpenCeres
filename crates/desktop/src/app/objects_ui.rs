//! The detailed report: students mark each moving object on every frame
//! (after the easy report's boxes), see it drawn as a track, and export the
//! search as JSON, with or without cutouts of what they saw. Teachers open
//! the same set and import students' reports to see their objects on the
//! images. The marks live in the search record ([`crate::history`]), so
//! they are saved as the student works and travel with history exports.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Label, Layout, Margin, Pos2, Rect, RichText,
    Stroke, pos2, vec2,
};
use openceres_core::align::Affine;

use super::history_ui::known_name;
use super::{App, KnownState, muted_wrapped};
use crate::history::{self, FrameRef, Record, Tagged, object_label};
use crate::load::{self, CutoutRequest};
use crate::theme::{self, S1, S2, S3, palette};
use crate::{png, update};

/// A click within this many pixels of a detected source snaps to its centre.
const SNAP_PIXELS: f64 = 6.0;
/// A marked object this close (on average) to a known object is that object.
/// Generous because known positions go through the header WCS, which is
/// several arcseconds off on IASC's Pan-STARRS chips.
const KNOWN_ARCSEC: f64 = 10.0;
/// Cutout sides in pixels: room around the whole track, within limits.
const CUTOUT_MIN: usize = 49;
const CUTOUT_MAX: usize = 161;
const CUTOUT_MARGIN: f64 = 24.0;

/// Another student's report of the open set, drawn over the images.
pub(super) struct Review {
    pub record: Record,
    /// Positions moved onto the open set's frames.
    pub objects: Vec<Tagged>,
    pub color: Color32,
    pub shown: bool,
}

/// An export with cutouts being written in the background.
pub(super) struct Export {
    pub rx: Receiver<Result<PathBuf, String>>,
}

impl App {
    /// The open set's frames, as search records refer to them.
    pub(super) fn frame_refs(&self) -> Vec<FrameRef> {
        self.set.as_ref().map_or_else(Vec::new, |s| {
            s.frames
                .iter()
                .map(|f| FrameRef {
                    file: f.name.clone(),
                    time: f.date_obs.clone(),
                })
                .collect()
        })
    }

    /// Shows `record` (another student's) over the open set, replacing an
    /// earlier copy of it.
    pub(super) fn add_review(&mut self, record: Record) {
        let ours = self.frame_refs();
        let map = history::frame_map(&record.frames, &ours);
        let objects = history::remap(&record.objects, &map, ours.len());
        self.reviews.retain(|r| r.record.id != record.id);
        let color = theme::REVIEW[self.reviews.len() % theme::REVIEW.len()];
        self.reviews.push(Review {
            record,
            objects,
            color,
            shown: true,
        });
    }

    // ------------------------------------------------------------------
    // Marking

    pub(super) fn start_marking(&mut self) {
        let n = self.frame_count();
        if n == 0 || self.marking.is_some() {
            return;
        }
        let label = (0..)
            .map(object_label)
            .find(|l| self.objects.iter().all(|o| &o.label != l))
            .unwrap_or_default();
        self.objects.push(Tagged {
            label,
            at: vec![None; n],
            ..Tagged::default()
        });
        self.marking = Some(self.objects.len() - 1);
        self.playing = false;
        self.index = 0;
        self.prefs.panel_open = true;
    }

    /// The student clicked at view pixel `(x, y)` while marking: put the
    /// object there on this frame (on the nearest detected source if one
    /// is close) and go on to the next frame still to mark.
    pub(super) fn place_mark(&mut self, x: f64, y: f64) {
        let Some(m) = self.marking else { return };
        let i = self.index;
        let (fx, fy) = self
            .placement(i)
            .inverse()
            .map_or((x, y), |t| t.apply(x, y));
        let snapped = self.current().and_then(|f| {
            f.stars
                .iter()
                .map(|s| (f64::from(s.x), f64::from(s.y)))
                .map(|(sx, sy)| (sx, sy, (sx - fx).hypot(sy - fy)))
                .filter(|s| s.2 <= SNAP_PIXELS)
                .min_by(|a, b| a.2.total_cmp(&b.2))
        });
        let at = snapped.map_or((fx, fy), |(sx, sy, _)| (sx, sy));
        if let Some(o) = self.objects.get_mut(m)
            && let Some(slot) = o.at.get_mut(i)
        {
            *slot = Some(at);
        }
        self.next_unmarked();
    }

    /// Uses the predicted position on this frame (Enter).
    pub(super) fn accept_prediction(&mut self) {
        if let Some((x, y)) = self.prediction() {
            self.place_mark(x, y);
        }
    }

    /// Goes to the next frame without a mark, or finishes.
    pub(super) fn next_unmarked(&mut self) {
        let Some(o) = self.marking.and_then(|m| self.objects.get(m)) else {
            return;
        };
        match (self.index + 1..o.at.len()).find(|&j| o.at[j].is_none()) {
            Some(j) => self.index = j,
            None => self.finish_marking(),
        }
    }

    /// Ends marking. An object marked on fewer than two frames says
    /// nothing about motion and is dropped.
    pub(super) fn finish_marking(&mut self) {
        let Some(m) = self.marking.take() else { return };
        let Some(o) = self.objects.get(m) else { return };
        if o.marked() < 2 {
            self.objects.remove(m);
            self.report_note = Some("An object needs a mark on at least two frames.".into());
            return;
        }
        self.report_note = None;
        let mut o = o.clone();
        self.derive(&mut o);
        if let Some(cell) = o.cell
            && !self.marks.contains(&cell)
        {
            self.marks.push(cell);
            self.copied = false;
        }
        self.objects[m] = o;
    }

    pub(super) fn delete_object(&mut self, i: usize) {
        if i < self.objects.len() {
            self.objects.remove(i);
            self.marking = match self.marking {
                Some(m) if m == i => None,
                Some(m) if m > i => Some(m - 1),
                m => m,
            };
        }
    }

    /// Box, motion and known-object match of a marked object.
    fn derive(&self, o: &mut Tagged) {
        let Some(set) = &self.set else { return };
        let marked: Vec<(usize, (f64, f64))> =
            o.at.iter()
                .enumerate()
                .filter_map(|(i, p)| Some((i, (*p)?)))
                .collect();
        o.cell = marked.first().and_then(|&(i, (x, y))| {
            let (x, y) = self.placement(i).apply(x, y);
            let (w, h) = self.view_size()?;
            let u = (x as f32 + 0.5) / w;
            let v = (h - 0.5 - y as f32) / h;
            Some(super::grid_box(u, v, self.grid_size))
        });

        // Motion from the first to the last mark, each through its own
        // frame's WCS (their errors largely cancel over so short a track).
        (o.rate, o.direction) = (None, None);
        if let (Some(&(a, pa)), Some(&(b, pb))) = (marked.first(), marked.last())
            && a != b
        {
            let (fa, fb) = (&set.frames[a], &set.frames[b]);
            if let (Some(wa), Some(wb), Some(ta), Some(tb)) =
                (&fa.wcs, &fb.wcs, fa.mid_jd, fb.mid_jd)
            {
                let (ra1, de1) = wa.pixel_to_sky(pa.0, pa.1);
                let (ra2, de2) = wb.pixel_to_sky(pb.0, pb.1);
                let minutes = (tb - ta) * 1440.0;
                if minutes.abs() > 1e-6 {
                    let (sep, pa_deg) = separation_and_angle(ra1, de1, ra2, de2);
                    o.rate = Some(sep * 3600.0 / minutes.abs());
                    o.direction = Some(pa_deg);
                }
            }
        }

        o.known = None;
        if let Some(KnownState::Done(k)) = &self.known {
            let scale = set
                .frames
                .first()
                .and_then(|f| f.wcs.as_ref())
                .map_or(0.25, |w| w.pixel_scale());
            o.known = k
                .objects
                .iter()
                .filter_map(|p| {
                    let d: Vec<f64> = marked
                        .iter()
                        .filter_map(|&(i, (x, y))| {
                            let (kx, ky) = p.at.get(i).copied().flatten()?;
                            Some((kx - x).hypot(ky - y) * scale)
                        })
                        .collect();
                    let mean = d.iter().sum::<f64>() / d.len().max(1) as f64;
                    (!d.is_empty() && mean <= KNOWN_ARCSEC).then_some((mean, p))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, p)| known_name(&p.object));
        }
    }

    /// Where the object being marked should be on this frame (view
    /// pixels), from a straight-line fit in time to its other marks.
    pub(super) fn prediction(&self) -> Option<(f64, f64)> {
        let o = self.objects.get(self.marking?)?;
        let set = self.set.as_ref()?;
        let time = |i: usize| set.frames.get(i).and_then(|f| f.mid_jd).unwrap_or(i as f64);
        let points: Vec<(f64, f64, f64)> =
            o.at.iter()
                .enumerate()
                .filter(|&(i, _)| i != self.index)
                .filter_map(|(i, p)| {
                    let (x, y) = (*p)?;
                    let (x, y) = self.placement(i).apply(x, y);
                    Some((time(i), x, y))
                })
                .collect();
        if points.len() < 2 {
            return None;
        }
        let n = points.len() as f64;
        let mt = points.iter().map(|p| p.0).sum::<f64>() / n;
        let var: f64 = points.iter().map(|p| (p.0 - mt).powi(2)).sum();
        if var <= 0.0 {
            return None;
        }
        let t = time(self.index) - mt;
        let fit = |k: fn(&(f64, f64, f64)) -> f64| {
            let mean = points.iter().map(k).sum::<f64>() / n;
            let slope = points
                .iter()
                .map(|p| (p.0 - mt) * (k(p) - mean))
                .sum::<f64>()
                / var;
            mean + slope * t
        };
        Some((fit(|p| p.1), fit(|p| p.2)))
    }

    // ------------------------------------------------------------------
    // Drawing

    /// Tracks of the student's own objects and of the shown reviews.
    pub(super) fn paint_objects(&self, painter: &egui::Painter, rect: Rect, origin: Pos2, h: f32) {
        let scale = self.view.scale;
        let to_screen = |i: usize, (x, y): (f64, f64)| {
            let (x, y) = self.placement(i).apply(x, y);
            origin + vec2(x as f32 + 0.5, h - 0.5 - y as f32) * scale
        };
        // Labels already drawn, so one on the same object goes below it.
        let taken = std::cell::RefCell::new(Vec::<Rect>::new());
        let draw = |o: &Tagged, color: Color32, name: Option<&str>, active: bool| {
            let points: Vec<(usize, Pos2)> =
                o.at.iter()
                    .enumerate()
                    .filter_map(|(i, p)| Some((i, to_screen(i, (*p)?))))
                    .collect();
            let line: Vec<Pos2> = points.iter().map(|p| p.1).collect();
            if line.len() > 1 {
                painter.add(egui::Shape::line(
                    line.clone(),
                    Stroke::new(1.0, color.gamma_multiply(0.6)),
                ));
            }
            for &(i, c) in &points {
                if i != self.index {
                    painter.circle_filled(c, 2.25, color.gamma_multiply(0.8));
                }
            }
            let Some(&(_, c)) = points.iter().find(|p| p.0 == self.index) else {
                return;
            };
            if !rect.expand(40.0).contains(c) {
                return;
            }
            let r = (3.0 * scale).max(9.0);
            let width = if active { 2.25 } else { 1.75 };
            painter.circle_stroke(c, r, Stroke::new(width + 2.0, theme::KNOWN_HALO));
            painter.circle_stroke(c, r, Stroke::new(width, color));
            let text = match name {
                Some(n) => format!("{} · {n}", o.label),
                None => o.label.clone(),
            };
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), color);
            let d = r * std::f32::consts::FRAC_1_SQRT_2;
            let mut chip = Rect::from_min_size(
                c + vec2(-d - 4.0 - galley.size().x - S2, -d - 15.0),
                vec2(galley.size().x + S2, 17.0),
            );
            let mut taken = taken.borrow_mut();
            while taken.iter().any(|t| t.intersects(chip)) {
                chip = chip.translate(vec2(0.0, 19.0));
            }
            taken.push(chip);
            painter.rect_filled(chip, 3.0, theme::GRID_CHIP);
            painter.galley(
                pos2(chip.min.x + S1, chip.center().y - galley.size().y / 2.0),
                galley,
                color,
            );
        };
        for review in self.reviews.iter().filter(|r| r.shown) {
            let name = review.record.searched_by.trim();
            let name = if name.is_empty() { "?" } else { name };
            for o in &review.objects {
                draw(o, review.color, Some(name), false);
            }
        }
        for (i, o) in self.objects.iter().enumerate() {
            draw(o, theme::TAG, None, Some(i) == self.marking);
        }
        if let Some((x, y)) = self.prediction() {
            let c = origin + vec2(x as f32 + 0.5, h - 0.5 - y as f32) * scale;
            let r = (3.0 * scale).max(9.0);
            let n = 16;
            for k in (0..n).step_by(2) {
                let a = |j: usize| j as f32 / n as f32 * std::f32::consts::TAU;
                let p = |j: usize| c + vec2(a(j).cos(), a(j).sin()) * r;
                painter.line_segment([p(k), p(k + 1)], Stroke::new(1.5, theme::TAG));
            }
        }
    }

    /// `Click A on frame 2 of 4` while marking, at the top of the stage.
    pub(super) fn marking_hint(&self, ui: &egui::Ui, rect: Rect) {
        let Some(o) = self.marking.and_then(|m| self.objects.get(m)) else {
            return;
        };
        let mut text = format!(
            "Click {} on frame {} of {}",
            o.label,
            self.index + 1,
            o.at.len()
        );
        if self.prediction().is_some() {
            text += "  ·  Enter: use the dashed ring";
        }
        text += "  ·  Esc: done";
        let painter = ui.painter_at(rect);
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.5), theme::OVERLAY_TEXT);
        let size = galley.size() + vec2(2.0 * S3, 2.0 * S1 + 2.0);
        let chip =
            Rect::from_center_size(pos2(rect.center().x, rect.min.y + S3 + size.y / 2.0), size);
        painter.rect_filled(chip, theme::RADIUS, theme::BADGE);
        painter.rect_stroke(
            chip,
            theme::RADIUS,
            Stroke::new(1.0, theme::TAG),
            egui::StrokeKind::Inside,
        );
        painter.galley(chip.min + vec2(S3, S1 + 1.0), galley, theme::OVERLAY_TEXT);
    }

    /// Centres the view on where object `o` is on this frame (or its first mark).
    fn focus_track(&mut self, o: &Tagged) {
        let Some((i, (x, y))) =
            o.at.get(self.index)
                .copied()
                .flatten()
                .map(|p| (self.index, p))
                .or_else(|| o.at.iter().enumerate().find_map(|(i, p)| Some((i, (*p)?))))
        else {
            return;
        };
        let (x, y) = self.placement(i).apply(x, y);
        let Some((_, h)) = self.view_size() else {
            return;
        };
        let v = &mut self.view;
        v.auto_fit = false;
        v.scale = v.scale.max(1.0);
        v.offset = v.stage / 2.0 - vec2(x as f32 + 0.5, h - 0.5 - y as f32) * v.scale;
    }

    // ------------------------------------------------------------------
    // Side panel

    /// "Objects": marking, the list of marked objects, and export.
    pub(super) fn objects_section(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = palette(ui);
        if self.set.is_none() {
            muted_wrapped(ui, "Open an image set first.");
            return;
        }
        if let Some(o) = self.marking.and_then(|m| self.objects.get(m)) {
            let (label, done) = (o.label.clone(), o.marked());
            Frame::new()
                .fill(p.inset)
                .corner_radius(theme::RADIUS)
                .stroke(Stroke::new(1.0, theme::TAG))
                .inner_margin(Margin::same(S2 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!(
                            "Marking {label}: click it on frame {} of {}.",
                            self.index + 1,
                            self.frame_count()
                        ))
                        .color(p.text),
                    );
                    muted_wrapped(
                        ui,
                        "Clicks snap to the centre of a detected source nearby. \
                         ← → choose another frame.",
                    );
                    ui.horizontal(|ui| {
                        if ui
                            .button("Not on this frame")
                            .on_hover_text("Too faint, hidden or off the image here")
                            .clicked()
                        {
                            self.next_unmarked();
                        }
                        let finish = ui
                            .add_enabled(done >= 2, Button::new("Done"))
                            .on_hover_text("Finish this object (Esc)")
                            .on_disabled_hover_text("Mark it on at least two frames");
                        if finish.clicked() {
                            self.finish_marking();
                        }
                        if done < 2 && ui.button("Cancel").clicked() {
                            self.finish_marking();
                        }
                    });
                });
        } else if self.objects.is_empty() {
            muted_wrapped(
                ui,
                "Mark each moving object on every frame. It is saved with your search \
                 and goes into the detailed report.",
            );
        }

        let mut delete = None;
        let mut focus = None;
        for (i, o) in self.objects.iter().enumerate() {
            if Some(i) == self.marking {
                continue;
            }
            let frames: String =
                o.at.iter()
                    .map(|p| if p.is_some() { '●' } else { '○' })
                    .collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S1;
                let text = RichText::new(o.summary()).color(theme::TAG);
                if ui
                    .add(Label::new(text).sense(egui::Sense::click()))
                    .on_hover_text("Show it")
                    .clicked()
                {
                    focus = Some(i);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Delete this object")
                        .clicked()
                    {
                        delete = Some(i);
                    }
                    ui.label(RichText::new(frames).small().color(p.muted))
                        .on_hover_text("Frames it is marked on");
                });
            });
            if let Some(k) = &o.known {
                ui.label(
                    RichText::new(format!("Known: {k}"))
                        .small()
                        .color(theme::KNOWN_TEXT),
                );
            }
        }
        if let Some(i) = focus {
            let o = self.objects[i].clone();
            self.focus_track(&o);
        }
        if let Some(i) = delete {
            self.delete_object(i);
        }

        let can_export = !self.objects.is_empty()
            && self.marking.is_none()
            && self.searches.record.is_some()
            && self.export.is_none();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S2;
            let w = (ui.available_width() - S2) / 2.0;
            let mark = ui
                .add_enabled(
                    self.marking.is_none(),
                    Button::new("Mark object").min_size(vec2(w, theme::TOOL)),
                )
                .on_hover_text("Click a moving object on each frame (M)");
            if mark.clicked() {
                self.start_marking();
            }
            ui.add_enabled_ui(can_export, |ui| {
                ui.menu_button("Export…", |ui| {
                    if ui
                        .button("Report (.json)")
                        .on_hover_text("Your search with each object's positions")
                        .clicked()
                    {
                        ui.close();
                        self.export_report(ui.ctx(), frame, false);
                    }
                    if ui
                        .button("Report with cutouts (.json)")
                        .on_hover_text(
                            "Also small pictures of each object on every frame, as you \
                             saw them, so your teacher can check without the images",
                        )
                        .clicked()
                    {
                        ui.close();
                        self.export_report(ui.ctx(), frame, true);
                    }
                })
                .response
                .on_hover_text("Save the detailed report to send to your teacher");
            });
        });
        if self.export.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0).color(p.muted));
                ui.label(RichText::new("Making cutouts…").small().color(p.muted));
            });
        }
        if let Some(e) = &self.report_error {
            ui.add(Label::new(RichText::new(e).small().color(p.danger)).wrap());
        } else if let Some(n) = &self.report_note {
            ui.add(Label::new(RichText::new(n).small().color(p.muted)).wrap());
        }
    }

    /// "Students' reports": import and show other students' objects.
    pub(super) fn reviews_section(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let p = palette(ui);
        if self.reviews.is_empty() {
            muted_wrapped(
                ui,
                "Import a student's report of this set to see the objects they marked.",
            );
        }
        let mut focus = None;
        let mut remove = None;
        for (ri, r) in self.reviews.iter_mut().enumerate() {
            let name = r.record.searched_by.trim();
            let name = if name.is_empty() { "Unnamed" } else { name };
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S1;
                ui.checkbox(&mut r.shown, RichText::new(name).color(r.color))
                    .on_hover_text("Show their objects on the images");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Stop showing this report (it stays in the history)")
                        .clicked()
                    {
                        remove = Some(ri);
                    }
                    ui.label(
                        RichText::new(match r.objects.len() {
                            1 => "1 object".to_owned(),
                            n => format!("{n} objects"),
                        })
                        .small()
                        .color(p.muted),
                    );
                });
            });
            for (oi, o) in r.objects.iter().enumerate() {
                let mut text = o.summary();
                if let Some(k) = &o.known {
                    text += &format!(" · {k}");
                }
                if ui
                    .add(
                        Label::new(RichText::new(text).small().color(p.text))
                            .sense(egui::Sense::click())
                            .wrap(),
                    )
                    .on_hover_text("Show it")
                    .clicked()
                {
                    focus = Some((ri, oi));
                }
            }
            if !r.record.notes.trim().is_empty() {
                ui.add(
                    Label::new(
                        RichText::new(format!("“{}”", r.record.notes.trim()))
                            .small()
                            .italics()
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }
        }
        if let Some((ri, oi)) = focus {
            self.reviews[ri].shown = true;
            let o = self.reviews[ri].objects[oi].clone();
            self.focus_track(&o);
        }
        if let Some(ri) = remove {
            self.reviews.remove(ri);
        }
        if ui
            .button("Import report…")
            .on_hover_text("Open a report a student exported for this image set")
            .clicked()
        {
            self.import_report(frame);
        }
        if let Some(n) = &self.review_note {
            ui.add(Label::new(RichText::new(n).small().color(p.muted)).wrap());
        }
    }

    // ------------------------------------------------------------------
    // Files

    fn export_report(&mut self, ctx: &egui::Context, frame: &eframe::Frame, cutouts: bool) {
        let Some(record) = &self.searches.record else {
            return;
        };
        let who = record.searched_by.trim();
        let mut name = format!(
            "{} {}",
            record.set,
            if who.is_empty() { "report" } else { who }
        );
        if cutouts {
            name += " with cutouts";
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("Export detailed report")
            .set_file_name(format!("{name}.json"))
            .add_filter("OpenCeres report", &["json"])
            .set_parent(frame)
            .save_file()
        else {
            return;
        };
        self.write_report(ctx, path, cutouts);
    }

    /// Writes the open search with its objects to `path`; with cutouts,
    /// in the background (the images are read again).
    pub(super) fn write_report(&mut self, ctx: &egui::Context, path: PathBuf, cutouts: bool) {
        let Some(mut record) = self.searches.record.clone() else {
            return;
        };
        record.objects.clone_from(&self.objects);
        record.boxes.clone_from(&self.marks);
        self.report_error = None;
        self.report_note = None;
        if !cutouts {
            match std::fs::write(&path, record.to_json("") + "\n") {
                Ok(()) => {
                    self.report_note = Some(saved_note(&path));
                    update::reveal(&path);
                }
                Err(e) => self.report_error = Some(format!("Couldn't save: {e}")),
            }
            return;
        }
        let Some(set) = &self.set else { return };
        let (requests, keys) = cutout_requests(set, &record.objects);
        let paths = set.paths.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = load::cutouts(&paths, &requests).and_then(|pictures| {
                let urls: Vec<((usize, usize), String)> = keys
                    .iter()
                    .zip(&requests)
                    .zip(pictures)
                    .filter_map(|((&key, r), px)| {
                        Some((key, png::data_url(&png::encode_gray(r.size, r.size, &px?))))
                    })
                    .collect();
                let text = record.to_json_with("", &|o, f| {
                    urls.iter()
                        .find(|(k, _)| *k == (o, f))
                        .map(|(_, u)| u.clone())
                });
                std::fs::write(&path, text + "\n")
                    .map(|()| path)
                    .map_err(|e| format!("Couldn't save: {e}"))
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.export = Some(Export { rx });
    }

    pub(super) fn poll_export(&mut self) {
        let Some(e) = &self.export else { return };
        let result = match e.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Making the cutouts failed.".into()),
        };
        self.export = None;
        match result {
            Ok(path) => {
                self.report_note = Some(saved_note(&path));
                update::reveal(&path);
            }
            Err(e) => self.report_error = Some(e),
        }
    }

    fn import_report(&mut self, frame: &eframe::Frame) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import a student's report")
            .add_filter("OpenCeres report", &["json"])
            .set_parent(frame)
            .pick_file()
        else {
            return;
        };
        let result = std::fs::read_to_string(&path)
            .map_err(|e| format!("Couldn't read {}: {e}", path.display()))
            .and_then(|t| history::read_records(&t));
        self.review_note = Some(match result {
            Ok(records) => self.show_reports(records),
            Err(e) => e,
        });
    }

    /// Shows the reports of the open set among `records` and keeps all of
    /// them in the history. Returns what happened, for the panel.
    pub(super) fn show_reports(&mut self, records: Vec<Record>) -> String {
        let Some(set) = &self.set else {
            return "Open the image set first.".into();
        };
        let set_name = set.name.clone();
        let observed = set.frames.first().and_then(|f| f.date_obs.clone());
        let own = self.searches.record.as_ref().map(|r| r.id.clone());
        if self.searches.enabled
            && let Some(dir) = self.searches.config.dir()
            && let Err(e) = history::import_records(&dir, records.clone())
        {
            self.searches.error = Some(e);
        }
        self.searches.list = None;
        let (mine, other): (Vec<Record>, Vec<Record>) = records
            .into_iter()
            .partition(|r| r.is_set(&set_name, observed.as_deref()));
        let mut shown = Vec::new();
        for r in mine.into_iter().filter(|r| Some(&r.id) != own.as_ref()) {
            shown.push(format!(
                "{} ({} objects)",
                if r.searched_by.trim().is_empty() {
                    "Unnamed"
                } else {
                    r.searched_by.trim()
                },
                r.objects.len()
            ));
            self.add_review(r);
        }
        let mut note = if shown.is_empty() {
            "Nothing in that file is for this image set.".to_owned()
        } else {
            format!("Showing {}.", shown.join(", "))
        };
        if let Some(r) = other.first() {
            note += &format!(
                " {} other search{} (e.g. {}) went into the history.",
                other.len(),
                if other.len() == 1 { "" } else { "es" },
                r.set
            );
        }
        note
    }
}

/// One cutout per marked position, all of an object's centred on the same
/// patch of sky (the middle of its track on the first frame's grid) so the
/// stars stand still and the object moves, as in the blink. Returns the
/// requests and their (object, frame) keys.
fn cutout_requests(
    set: &super::Set,
    objects: &[Tagged],
) -> (Vec<CutoutRequest>, Vec<(usize, usize)>) {
    let to_ref = |i: usize| {
        set.frames
            .get(i)
            .and_then(|f| f.to_reference)
            .unwrap_or(Affine::IDENTITY)
    };
    let mut requests = Vec::new();
    let mut keys = Vec::new();
    for (oi, o) in objects.iter().enumerate() {
        let track: Vec<(f64, f64)> =
            o.at.iter()
                .enumerate()
                .filter_map(|(i, p)| p.map(|(x, y)| to_ref(i).apply(x, y)))
                .collect();
        if track.is_empty() {
            continue;
        }
        let (min, max) = track.iter().fold(
            ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
            |(lo, hi), &(x, y)| ((lo.0.min(x), lo.1.min(y)), (hi.0.max(x), hi.1.max(y))),
        );
        let centre = ((min.0 + max.0) / 2.0, (min.1 + max.1) / 2.0);
        let span = (max.0 - min.0).max(max.1 - min.1);
        let size = ((span + 2.0 * CUTOUT_MARGIN).ceil() as usize).clamp(CUTOUT_MIN, CUTOUT_MAX) | 1;
        for (fi, p) in o.at.iter().enumerate() {
            if p.is_none() {
                continue;
            }
            let Some(frame) = set.frames.get(fi) else {
                continue;
            };
            let (x, y) = to_ref(fi)
                .inverse()
                .map_or(centre, |t| t.apply(centre.0, centre.1));
            requests.push(CutoutRequest {
                file: frame.name.clone(),
                x,
                y,
                size,
            });
            keys.push((oi, fi));
        }
    }
    (requests, keys)
}

/// `Saved XY57_p01 Ayşe.json (shown in Finder)`: the folder is long and the
/// file is revealed anyway.
fn saved_note(path: &std::path::Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let shown = if cfg!(target_os = "macos") {
        " (shown in Finder)"
    } else {
        ""
    };
    format!("Saved {name}{shown}")
}

/// Angular separation (degrees) from the first position to the second and
/// its position angle (degrees east of north, 0–360).
fn separation_and_angle(ra1: f64, de1: f64, ra2: f64, de2: f64) -> (f64, f64) {
    let (ra1, de1, ra2, de2) = (
        ra1.to_radians(),
        de1.to_radians(),
        ra2.to_radians(),
        de2.to_radians(),
    );
    let dra = ra2 - ra1;
    // Haversine for the separation, robust for tiny angles.
    let a = ((de2 - de1) / 2.0).sin().powi(2) + de1.cos() * de2.cos() * (dra / 2.0).sin().powi(2);
    let sep = 2.0 * a.sqrt().min(1.0).asin();
    let y = dra.sin() * de2.cos();
    let x = de1.cos() * de2.sin() - de1.sin() * de2.cos() * dra.cos();
    (sep.to_degrees(), y.atan2(x).to_degrees().rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separation_and_position_angle() {
        // 10″ due north, then due east (at Dec 0), then south-west.
        let s = 10.0 / 3600.0;
        let (d, pa) = separation_and_angle(20.0, 0.0, 20.0, s);
        assert!((d - s).abs() < 1e-12 && pa.abs() < 1e-6, "{d} {pa}");
        let (_, pa) = separation_and_angle(20.0, 0.0, 20.0 + s, 0.0);
        assert!((pa - 90.0).abs() < 1e-6);
        let (_, pa) = separation_and_angle(20.0, 4.0, 20.0 - s, 4.0 - s);
        assert!((pa - 225.0).abs() < 0.1, "{pa}");
    }
}
