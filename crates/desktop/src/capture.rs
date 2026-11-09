//! Developer-only window screenshots, for reviewing the UI without
//! capturing the rest of the screen.
//!
//! ```sh
//! OPENCERES_SCREENSHOT=/tmp/out.png \
//! OPENCERES_SCREENSHOT_STATE=dark,grid,mark=2x1,panel,paused,size=1280x820 \
//!   openceres-desktop data/set.zip
//! ```
//!
//! With `OPENCERES_SCREENSHOT` set, the app starts with default preferences
//! (and never saves them), opens the paths given on the command line, waits
//! until they have loaded and a few frames have been drawn, saves a PNG of
//! its own window and exits. `OPENCERES_SCREENSHOT_STATE` is a comma list of:
//! `light`, `dark`, `stars`, `invert`, `noalign` (frames as stored), `grid`, `panel`, `settings` (or
//! `settings=PAGE`, e.g. `settings=updates`), `search=TEXT` (in Settings),
//! `update=VERSION` (pretend that version is available), `offers` (the box
//! offering updates; use with `update=`), `help`, `history`
//! or `history=SET` (the history window, with SET's search opened;
//! screenshots leave the history alone unless `OPENCERES_HISTORY_DIR` gives
//! them a folder), `paused`, `frame=N`
//! (1-based), `mark=RxC` (row × column, repeatable), `gridsize=N`, `known`
//! (look up known objects and wait for the answer), `focus=LABEL` (centre
//! and zoom on that known object, where it is on the first frame),
//! `objects=FILE` (a report's objects as the student's own), `marking`
//! (continue marking the last of them), `report=FILE` (import a student's
//! report to review), `export=FILE` (write the detailed report with cutouts
//! there; needs `OPENCERES_HISTORY_DIR`), `delete=N`, `edit=N` (delete or edit
//! the Nth of the objects, 1-based), `clear` (the Clear button), `askgrid=N`
//! (ask to change the grid to N), `snap=source|clicked` (last placement shown
//! while marking), `expired` (a made-up team match whose session ended) and
//! `size=WxH` (window size in points).

use eframe::egui;

#[derive(Debug, Default, PartialEq)]
pub struct State {
    pub theme: Option<egui::Theme>,
    pub stars: bool,
    pub invert: bool,
    pub no_align: bool,
    pub grid: bool,
    pub panel: bool,
    pub settings: bool,
    pub settings_page: Option<String>,
    pub search: Option<String>,
    pub update: Option<String>,
    pub offers: bool,
    pub help: bool,
    /// The history window, with the first search of this set opened ("" = none).
    pub history: Option<String>,
    pub paused: bool,
    pub known: bool,
    pub focus: Option<String>,
    pub frame: Option<usize>,
    pub grid_size: Option<usize>,
    /// Report boxes as (row, column).
    pub marks: Vec<(usize, usize)>,
    pub objects: Option<String>,
    pub marking: bool,
    pub report: Option<String>,
    pub export: Option<String>,
    pub delete: Option<usize>,
    pub edit: Option<usize>,
    pub clear: bool,
    pub ask_grid: Option<usize>,
    pub snap: Option<String>,
    pub expired: bool,
    pub size: Option<[f32; 2]>,
}

pub struct Capture {
    pub path: String,
    pub state: State,
    /// Frames drawn since the image set (if any) finished loading.
    pub frames: u32,
    /// Input time when the state was applied; fade-in animations must end
    /// before the shot.
    pub applied_at: Option<f64>,
    pub requested: bool,
    /// Known object to centre on once the lookup has finished.
    pub focus: Option<String>,
}

impl Capture {
    pub fn from_env() -> Option<Capture> {
        let path = std::env::var("OPENCERES_SCREENSHOT")
            .ok()
            .filter(|p| !p.is_empty())?;
        let state = State::parse(&std::env::var("OPENCERES_SCREENSHOT_STATE").unwrap_or_default());
        Some(Capture {
            path,
            state,
            frames: 0,
            applied_at: None,
            requested: false,
            focus: None,
        })
    }
}

impl State {
    pub fn parse(list: &str) -> State {
        let mut s = State::default();
        let pair = |v: &str| {
            let (a, b) = v.split_once('x')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        };
        for item in list.split(',').map(str::trim) {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            match key {
                "light" => s.theme = Some(egui::Theme::Light),
                "dark" => s.theme = Some(egui::Theme::Dark),
                "stars" => s.stars = true,
                "invert" => s.invert = true,
                "noalign" => s.no_align = true,
                "grid" => s.grid = true,
                "panel" => s.panel = true,
                "settings" => {
                    s.settings = true;
                    s.settings_page = Some(value.to_owned()).filter(|v| !v.is_empty());
                }
                "search" => s.search = Some(value.to_owned()),
                "update" => s.update = Some(value.to_owned()).filter(|v| !v.is_empty()),
                "offers" => s.offers = true,
                "help" => s.help = true,
                "history" => s.history = Some(value.to_owned()),
                "paused" => s.paused = true,
                "known" => s.known = true,
                "focus" => s.focus = Some(value.to_owned()).filter(|v| !v.is_empty()),
                "frame" => s.frame = value.parse().ok(),
                "gridsize" => s.grid_size = value.parse().ok(),
                "mark" => s.marks.extend(pair(value)),
                "objects" => s.objects = Some(value.to_owned()),
                "marking" => s.marking = true,
                "report" => s.report = Some(value.to_owned()),
                "export" => s.export = Some(value.to_owned()),
                "delete" => s.delete = value.parse().ok(),
                "edit" => s.edit = value.parse().ok(),
                "clear" => s.clear = true,
                "askgrid" => s.ask_grid = value.parse().ok(),
                "snap" => s.snap = Some(value.to_owned()),
                "expired" => s.expired = true,
                "size" => {
                    s.size = pair(value).map(|(w, h)| [w as f32, h as f32]);
                }
                _ => {}
            }
        }
        s
    }
}

pub fn write_png(path: &str, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    std::fs::write(path, crate::png::encode_rgba(w, h, &rgba(image)))
}

fn rgba(image: &egui::ColorImage) -> Vec<u8> {
    image
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state() {
        let s = State::parse(
            "dark, grid,mark=2x1,mark=3x4,size=900x600,frame=2,known,focus=2026 RA34,bogus",
        );
        assert_eq!(s.theme, Some(egui::Theme::Dark));
        assert!(s.grid && !s.panel);
        assert_eq!(s.marks, [(2, 1), (3, 4)]);
        assert_eq!(s.size, Some([900.0, 600.0]));
        assert_eq!(s.frame, Some(2));
        assert!(s.known);
        assert_eq!(s.focus.as_deref(), Some("2026 RA34"));
    }
}
