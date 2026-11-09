//! The box offering what the startup checks found: a new OpenCeres and a
//! newer MPCORB catalogue. It floats over a corner of the image and is not a
//! modal, so the app stays usable while it asks and while the downloads,
//! which run on their own threads, are in progress.

use eframe::egui::{self, Align2, Button, Frame, Id, Label, Order, Rect, RichText, Stroke, vec2};

use super::App;
use crate::theme::{S1, S2, S3, palette};
use crate::update;

const WIDTH: f32 = 300.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Offer {
    /// Nothing to offer, or the user answered "Not now" or closed it.
    #[default]
    Hidden,
    Asking,
    /// The user said yes; the row follows the download.
    Accepted,
}

#[derive(Default)]
pub(super) struct Offers {
    pub app: Offer,
    pub catalogue: Offer,
}

/// Buttons of a row; the first is the highlighted one.
fn buttons(ui: &mut egui::Ui, first: &str, second: &str) -> (bool, bool) {
    let p = palette(ui);
    ui.add_space(S1);
    ui.horizontal(|ui| {
        let main = Button::new(RichText::new(first).color(p.accent_text))
            .fill(p.accent)
            .stroke(Stroke::NONE);
        (ui.add(main).clicked(), ui.button(second).clicked())
    })
    .inner
}

fn text(ui: &mut egui::Ui, text: impl Into<String>, color: egui::Color32) {
    ui.add(Label::new(RichText::new(text.into()).color(color)).wrap());
}

impl App {
    /// Draws the box in the bottom-right corner of `stage`, if it has
    /// anything to say.
    pub(super) fn offers_box(&mut self, ctx: &egui::Context, stage: Rect) {
        let (app, catalogue) = (self.offers.app, self.offers.catalogue);
        if app == Offer::Hidden && catalogue == Offer::Hidden {
            return;
        }
        egui::Area::new(Id::new("offers"))
            .order(Order::Foreground)
            .fixed_pos(stage.right_bottom() - vec2(S3, S3))
            .pivot(Align2::RIGHT_BOTTOM)
            .show(ctx, |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(WIDTH);
                    if app != Offer::Hidden {
                        self.app_offer(ui);
                    }
                    if app != Offer::Hidden && catalogue != Offer::Hidden {
                        ui.add_space(S2);
                        ui.separator();
                        ui.add_space(S2);
                    }
                    if catalogue != Offer::Hidden {
                        self.catalogue_offer(ui);
                    }
                });
            });
    }

    fn app_offer(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        let Some(release) = self.updates.found.clone() else {
            self.offers.app = Offer::Hidden; // a later check found nothing new
            return;
        };
        if self.offers.app == Offer::Asking {
            let size = release.asset.as_ref().map_or(String::new(), |a| {
                format!(" ({:.0} MB)", a.size as f64 / 1e6)
            });
            text(
                ui,
                format!(
                    "A new version of OpenCeres ({}) is available.",
                    release.version
                ),
                p.text,
            );
            text(ui, format!("Would you like to download it{size}?"), p.muted);
            let (yes, no) = buttons(ui, "Download", "Not now");
            if yes {
                if release.asset.is_some() {
                    self.start_update_download(ui.ctx());
                    self.offers.app = Offer::Accepted;
                } else {
                    // No build for this computer: the release page has the rest.
                    update::open_url(&release.page);
                    self.offers.app = Offer::Hidden;
                }
            } else if no {
                self.offers.app = Offer::Hidden;
            }
        } else if self.updates.downloading.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(13.0).color(p.muted));
                text(
                    ui,
                    format!("Downloading OpenCeres {}…", release.version),
                    p.muted,
                );
            });
        } else if let Some(path) = self.updates.downloaded.clone() {
            text(
                ui,
                format!("OpenCeres {} is downloaded.", release.version),
                p.text,
            );
            let (open, close) = buttons(ui, "Open", "Close");
            if open {
                update::reveal(&path);
            }
            if open || close {
                self.offers.app = Offer::Hidden;
            }
        } else {
            let error = self.updates.download_error.clone().unwrap_or_default();
            text(ui, error, p.danger);
            let (retry, close) = buttons(ui, "Try again", "Close");
            if retry {
                self.start_update_download(ui.ctx());
            } else if close {
                self.offers.app = Offer::Hidden;
            }
        }
    }

    fn catalogue_offer(&mut self, ui: &mut egui::Ui) {
        let p = palette(ui);
        if self.offers.catalogue == Offer::Asking {
            text(ui, "A newer MPCORB catalogue is available.", p.text);
            text(ui, "Would you like to download it (about 94 MB)?", p.muted);
            let (yes, no) = buttons(ui, "Download", "Not now");
            if yes {
                self.start_download(ui.ctx());
                self.offers.catalogue = Offer::Accepted;
            } else if no {
                self.offers.catalogue = Offer::Hidden;
            }
        } else if self.download.is_some() {
            text(ui, "Downloading the MPCORB catalogue…", p.muted);
            ui.add_space(S1);
            self.download_controls(ui);
        } else if let Some(error) = self.download_error.clone() {
            text(ui, error, p.danger);
            let (retry, close) = buttons(ui, "Try again", "Close");
            if retry {
                self.start_download(ui.ctx());
            } else if close {
                self.offers.catalogue = Offer::Hidden;
            }
        } else {
            text(ui, "The MPCORB catalogue is up to date.", p.text);
            ui.add_space(S1);
            if ui.button("Close").clicked() {
                self.offers.catalogue = Offer::Hidden;
            }
        }
    }
}
