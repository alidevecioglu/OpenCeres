//! The settings registry: every setting is declared once, in [`ENTRIES`].
//! The Settings window draws its pages from it, the search box filters it,
//! the preferences file is read and written through it, and "Reset this
//! section" / "Reset all" put its values back to [`Prefs::default`].
//!
//! Adding a setting: add a field to [`Prefs`] (and its default), then one
//! [`Entry`] here. A `Toggle`, `Choice` or `Seconds` is saved under its `id`; a
//! `Custom` entry draws itself (status rows, buttons, tables) and is not
//! saved by the registry.

use eframe::egui;

use crate::app::{App, history_ui, mpc_ui, onboarding_ui, settings_ui, team_ui};
use crate::icons::Icon;
use crate::prefs::{GRID_SIZES, KnownSource, Prefs, ThemeChoice};
use crate::speed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    General,
    Appearance,
    Viewing,
    Features,
    Report,
    ReportGrid,
    Known,
    History,
    Team,
    Updates,
    Shortcuts,
    About,
}

impl Category {
    pub const ALL: [Category; 12] = [
        Category::General,
        Category::Appearance,
        Category::Viewing,
        Category::Features,
        Category::Report,
        Category::ReportGrid,
        Category::Known,
        Category::History,
        Category::Team,
        Category::Updates,
        Category::Shortcuts,
        Category::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Viewing => "Viewing",
            Category::Features => "Features",
            Category::Report => "Observer & report",
            Category::ReportGrid => "Report grid",
            Category::Known => "Known objects",
            Category::History => "History",
            Category::Team => "Team",
            Category::Updates => "Updates",
            Category::Shortcuts => "Shortcuts",
            Category::About => "About",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Category::General => Icon::Gear,
            Category::Appearance => Icon::Theme,
            Category::Viewing => Icon::Eye,
            Category::Features => Icon::Check,
            Category::Report => Icon::Report,
            Category::ReportGrid => Icon::Grid,
            Category::Known => Icon::Known,
            Category::History => Icon::History,
            Category::Team => Icon::Team,
            Category::Updates => Icon::Update,
            Category::Shortcuts => Icon::Keyboard,
            Category::About => Icon::Info,
        }
    }

    /// Whether the page is listed: the report grid's page only while the
    /// box report is turned on.
    pub fn visible(self, p: &Prefs) -> bool {
        self != Category::ReportGrid || p.box_report
    }

    /// For `OPENCERES_SCREENSHOT_STATE=settings=<key>`: the label in lower
    /// case without spaces, e.g. `reportgrid`.
    pub fn from_key(key: &str) -> Option<Category> {
        Category::ALL
            .into_iter()
            .find(|c| c.label().replace(' ', "").eq_ignore_ascii_case(key))
    }
}

/// One option of a [`Kind::Choice`]: saved as `key`, shown as `label`.
pub struct Opt {
    pub key: &'static str,
    pub label: &'static str,
}

pub enum Kind {
    Toggle {
        get: fn(&Prefs) -> bool,
        set: fn(&mut Prefs, bool),
    },
    Choice {
        options: &'static [Opt],
        get: fn(&Prefs) -> usize,
        set: fn(&mut Prefs, usize),
        /// Numeric options: a value not in the list reads as the nearest one.
        nearest: bool,
    },
    /// A blink speed in seconds, any value in [`speed::MIN`]–[`speed::MAX`],
    /// saved rounded to 0.01 s.
    Seconds {
        get: fn(&Prefs) -> f32,
        set: fn(&mut Prefs, f32),
    },
    Custom(fn(&mut App, &mut egui::Ui)),
}

pub struct Entry {
    /// Stable name; also the key in the preferences file.
    pub id: &'static str,
    pub category: Category,
    pub label: &'static str,
    pub description: &'static str,
    /// Extra words the search should find this entry by.
    pub keywords: &'static str,
    pub kind: Kind,
}

impl Entry {
    /// Saved in the preferences file and covered by the Reset buttons.
    pub fn is_value(&self) -> bool {
        !matches!(self.kind, Kind::Custom(_))
    }

    /// Every word of `query` appears in the label, description, keywords
    /// or category name (case-insensitive).
    pub fn matches(&self, query: &str) -> bool {
        let text = format!(
            "{} {} {} {}",
            self.label,
            self.description,
            self.keywords,
            self.category.label()
        )
        .to_lowercase();
        let query = query.to_lowercase();
        let mut words = query.split_whitespace().peekable();
        words.peek().is_some() && words.all(|w| text.contains(w))
    }
}

const fn opt(key: &'static str, label: &'static str) -> Opt {
    Opt { key, label }
}

const THEMES: [Opt; 3] = [
    opt("system", "System"),
    opt("light", "Light"),
    opt("dark", "Dark"),
];
const GRID_OPTIONS: [Opt; 5] = [
    opt("3", "3×3"),
    opt("4", "4×4"),
    opt("5", "5×5"),
    opt("6", "6×6"),
    opt("8", "8×8"),
];
const SOURCES: [Opt; 3] = [
    opt("auto", "Automatic"),
    opt("skybot", "SkyBoT"),
    opt("mpcorb", "MPCORB"),
];
/// Automatic, then [`openceres_core::gaia::MIRRORS`] in order.
const MIRROR_OPTIONS: [Opt; 6] = [
    opt("auto", "Automatic"),
    opt("cds", "CDS Strasbourg"),
    opt("cfa", "CfA Harvard"),
    opt("idia", "IDIA Cape Town"),
    opt("naoj", "NAOJ Tokyo"),
    opt("china-vo", "China-VO Beijing"),
];

fn index_of<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

pub static ENTRIES: &[Entry] = &[
    // General
    Entry {
        id: "language",
        category: Category::General,
        label: "Language",
        description: "The language of OpenCeres's windows.",
        keywords: "language translation translate locale english",
        kind: Kind::Custom(onboarding_ui::language_setting),
    },
    Entry {
        id: "welcome",
        category: Category::General,
        label: "Welcome window",
        description: "The short tour shown the first time OpenCeres starts.",
        keywords: "welcome onboarding first run tour introduction tips start",
        kind: Kind::Custom(onboarding_ui::welcome_setting),
    },
    Entry {
        id: "data_folder",
        category: Category::General,
        label: "Data folder",
        description: "Preferences and the offline MPCORB catalogue are kept here.",
        keywords: "location path directory finder explorer files storage",
        kind: Kind::Custom(settings_ui::data_folder),
    },
    Entry {
        id: "reset_all",
        category: Category::General,
        label: "Reset all settings",
        description: "Puts every setting on every page back to its default.",
        keywords: "defaults restore factory clear",
        kind: Kind::Custom(settings_ui::reset_all),
    },
    // Appearance
    Entry {
        id: "theme",
        category: Category::Appearance,
        label: "Theme",
        description: "System follows the light or dark mode of your computer.",
        keywords: "dark mode light mode colours colors appearance night",
        kind: Kind::Choice {
            options: &THEMES,
            get: |p| index_of(&ThemeChoice::ALL, &p.theme),
            set: |p, i| p.theme = ThemeChoice::ALL[i],
            nearest: false,
        },
    },
    // Viewing
    Entry {
        id: "blink_seconds",
        category: Category::Viewing,
        label: "Blink speed",
        description: "Seconds each frame is shown when a set opens, 0.10 to 5.00. \
                      In the toolbar, drag or click the value, or press , and . \
                      (Shift for bigger steps) to change it for now.",
        keywords: "speed seconds frame rate interval slower faster animation tortoise hare \
                   slider",
        kind: Kind::Seconds {
            get: |p| p.blink_seconds,
            set: |p, v| p.blink_seconds = v,
        },
    },
    Entry {
        id: "show_stars",
        category: Category::Viewing,
        label: "Show detected stars",
        description: "Ring the stars found in each image when a set opens. S switches it for now.",
        keywords: "stars detection rings circles sources overlay",
        kind: Kind::Toggle {
            get: |p| p.show_stars,
            set: |p, v| p.show_stars = v,
        },
    },
    Entry {
        id: "invert",
        category: Category::Viewing,
        label: "Invert",
        description: "Dark stars on a white sky when a set opens. I switches it for now.",
        keywords: "negative white background black stars contrast",
        kind: Kind::Toggle {
            get: |p| p.invert,
            set: |p, v| p.invert = v,
        },
    },
    Entry {
        id: "align",
        category: Category::Viewing,
        label: "Align frames on the stars",
        description: "Matches the stars of every frame to the first, so they stand still \
                      in the blink even when the telescope drifted between exposures and \
                      only moving objects jump. A switches it for now.",
        keywords: "align register match stars guiding drift shift stack blink steady \
                   wobble jump",
        kind: Kind::Toggle {
            get: |p| p.align,
            set: |p, v| p.align = v,
        },
    },
    // Features
    Entry {
        id: "box_report",
        category: Category::Features,
        label: "Box report",
        description: "The older way to report: a grid over the image, and you click \
                      each box where something moves. Turning it on adds the grid \
                      button (G), the Easy report in the side panel and the Report \
                      grid settings.",
        keywords: "grid boxes easy report simple 1x1 old legacy feature enable",
        kind: Kind::Toggle {
            get: |p| p.box_report,
            set: |p, v| p.box_report = v,
        },
    },
    // Observer & report
    Entry {
        id: "report_code",
        category: Category::Report,
        label: "Observatory code",
        description: "The MPC code of the telescope that took the images. Leave it empty \
                      to take it from the images (F51 for Pan-STARRS 1).",
        keywords: "mpc cod observatory code station site f51 pan-starrs",
        kind: Kind::Custom(mpc_ui::code_setting),
    },
    Entry {
        id: "report_observers",
        category: Category::Report,
        label: "Observers",
        description: "Who took the images, as initials and surname, separated by commas \
                      (e.g. “J. R. Smith, A. Jones”). Ask your campaign what to put here.",
        keywords: "obs observers names initials surname team campaign",
        kind: Kind::Custom(mpc_ui::observers_setting),
    },
    Entry {
        id: "history_name",
        category: Category::Report,
        label: "Measured by",
        description: "Filled in for new searches. Each search keeps its own (in the side \
                      panel and the report), so on a shared computer everyone can type theirs.",
        keywords: "mea measurer student name searched by your name person profile",
        kind: Kind::Custom(history_ui::name_setting),
    },
    Entry {
        id: "report_contact",
        category: Category::Report,
        label: "Contact",
        description: "One person the MPC can write to: name, address and e-mail.",
        keywords: "con contact email address teacher leader",
        kind: Kind::Custom(mpc_ui::contact_setting),
    },
    Entry {
        id: "report_telescope",
        category: Category::Report,
        label: "Telescope",
        description: "Aperture, focal ratio, design and detector (no camera brands). \
                      Pan-STARRS 1: 1.8 m, f/4.4, Ritchey-Chretien, CCD.",
        keywords: "tel telescope aperture focal ratio design ccd detector",
        kind: Kind::Custom(mpc_ui::telescope_setting),
    },
    Entry {
        id: "report_designations",
        category: Category::Report,
        label: "Designation prefix",
        description: "New objects are named with this and a number, e.g. OC0001. Letters \
                      and digits, starting with a letter; agree on one per team so names \
                      don't repeat.",
        keywords: "temporary designation prefix name number tracklet trksub",
        kind: Kind::Custom(mpc_ui::prefix_setting),
    },
    Entry {
        id: "report_ack",
        category: Category::Report,
        label: "Acknowledgement",
        description: "Text the MPC e-mails back when it receives the report (ACK), and \
                      where to (AC2). Optional.",
        keywords: "ack ac2 acknowledgement receipt email batch",
        kind: Kind::Custom(mpc_ui::ack_setting),
    },
    Entry {
        id: "report_header",
        category: Category::Report,
        label: "Report header",
        description: "The first lines of every report, as they will be sent.",
        keywords: "header preview cod obs mea tel net lines",
        kind: Kind::Custom(mpc_ui::header_preview),
    },
    // Report grid
    Entry {
        id: "show_grid",
        category: Category::ReportGrid,
        label: "Show the grid",
        description: "Draw the report grid when a set opens. G switches it for now.",
        keywords: "boxes cells show on open default report",
        kind: Kind::Toggle {
            get: |p| p.show_grid,
            set: |p, v| p.show_grid = v,
        },
    },
    Entry {
        id: "grid_size",
        category: Category::ReportGrid,
        label: "Grid size",
        description: "Boxes across and down. Changing it clears the marked boxes.",
        keywords: "boxes cells rows columns easy report iasc",
        kind: Kind::Choice {
            options: &GRID_OPTIONS,
            get: |p| index_of(&GRID_SIZES, &p.grid_size),
            set: |p, i| p.grid_size = GRID_SIZES[i],
            nearest: false,
        },
    },
    Entry {
        id: "grid_names",
        category: Category::ReportGrid,
        label: "How boxes are named",
        description: "Row × column, counted from the top-left of the image as shown: \
                      1x2 is the top row, second column. The report lists them in \
                      reading order.",
        keywords: "row column top-left naming convention label 1x2",
        kind: Kind::Custom(settings_ui::grid_names),
    },
    // Known objects
    Entry {
        id: "show_known",
        category: Category::Known,
        label: "Show known objects",
        description: "Look up and mark known asteroids and comets when a set opens. \
                      K switches it for now.",
        keywords: "skybot lookup default on open asteroids comets mark",
        kind: Kind::Toggle {
            get: |p| p.show_known,
            set: |p, v| p.show_known = v,
        },
    },
    Entry {
        id: "known_source",
        category: Category::Known,
        label: "Source",
        description: "SkyBoT asks IMCCE's service online. MPCORB searches the Minor \
                      Planet Center's asteroid and comet orbits on this computer once \
                      downloaded. Automatic uses MPCORB when it is here, else SkyBoT.",
        keywords: "skybot mpcorb automatic online offline asteroids comets lookup imcce mpc",
        kind: Kind::Choice {
            options: &SOURCES,
            get: |p| index_of(&KnownSource::ALL, &p.known_source),
            set: |p, i| p.known_source = KnownSource::ALL[i],
            nearest: false,
        },
    },
    Entry {
        id: "mpcorb",
        category: Category::Known,
        label: "MPCORB catalogue",
        description: "About 94 MB from the Minor Planet Center, with its comet orbits \
                      (CometEls.txt). Check for update asks the server without downloading.",
        keywords: "download update catalogue catalog offline orbit database mpc check \
                   comets cometels",
        kind: Kind::Custom(settings_ui::mpcorb),
    },
    Entry {
        id: "gaia_mirror",
        category: Category::Known,
        label: "Gaia star catalogue",
        description: "Reports are measured against Gaia DR3 stars from VizieR. \
                      Automatic tries the mirror that last worked, then the others.",
        keywords: "gaia vizier mirror server catalogue catalog plate solution astrometry \
                   cds cfa idia naoj china",
        kind: Kind::Choice {
            options: &MIRROR_OPTIONS,
            get: |p| p.gaia_mirror.min(MIRROR_OPTIONS.len() - 1),
            set: |p, i| p.gaia_mirror = i,
            nearest: false,
        },
    },
    Entry {
        id: "gaia_status",
        category: Category::Known,
        label: "VizieR mirrors",
        description: "Test all asks each mirror for a few stars.",
        keywords: "gaia vizier mirror test status retest online offline",
        kind: Kind::Custom(mpc_ui::mirror_status),
    },
    // History
    Entry {
        id: "history_folder",
        category: Category::History,
        label: "History folder",
        description: "Each search is saved here as a small file of its own. Choose a \
                      shared folder (OneDrive, Google Drive or a network drive) to \
                      collect a whole class's searches in one place.",
        keywords: "folder location shared class teacher onedrive google drive network sync",
        kind: Kind::Custom(history_ui::folder_setting),
    },
    Entry {
        id: "history_window",
        category: Category::History,
        label: "Search history",
        description: "Every search, its status and notes. Export it for a teacher or \
                      import a class's searches. H opens it from the main window.",
        keywords: "history searches export import csv json teacher finds reported",
        kind: Kind::Custom(history_ui::open_history),
    },
    // Team
    Entry {
        id: "team_server",
        category: Category::Team,
        label: "Team server",
        description: "An OpenCeres Team server, if your teacher or club uses one: \
                      campaigns, assigned sets and everyone's findings. Leave it empty \
                      to keep OpenCeres fully offline.",
        keywords: "server cloud self-hosted url address backend campaign class teacher \
                   online offline connection test",
        kind: Kind::Custom(team_ui::server_setting),
    },
    Entry {
        id: "team_account",
        category: Category::Team,
        label: "Account",
        description: "Sign in with the account you have on the team server. Only the \
                      server address and the session are kept, on this computer.",
        keywords: "sign in log in login sign out logout email password account user \
                   change password",
        kind: Kind::Custom(team_ui::account_setting),
    },
    // Updates
    Entry {
        id: "update_check",
        category: Category::Updates,
        label: "Check for updates automatically",
        description: "Each time OpenCeres starts, asks GitHub whether a newer version is \
                      out. Nothing is downloaded without asking.",
        keywords: "update new version release github automatic",
        kind: Kind::Toggle {
            get: |p| p.update_check,
            set: |p, v| p.update_check = v,
        },
    },
    Entry {
        id: "update_status",
        category: Category::Updates,
        label: "OpenCeres version",
        description: "",
        keywords: "update version check now download release notes what's new",
        kind: Kind::Custom(settings_ui::update_status),
    },
    // Shortcuts
    Entry {
        id: "shortcuts",
        category: Category::Shortcuts,
        label: "Keyboard shortcuts",
        description: "Press ? in the main window to see these at any time.",
        keywords: "keys keyboard hotkeys space arrows tab",
        kind: Kind::Custom(settings_ui::shortcuts),
    },
    // About
    Entry {
        id: "about",
        category: Category::About,
        label: "OpenCeres",
        description: "",
        keywords: "about version licence license mit apache github credits skybot mpc \
                   vsop87 jpl source code",
        kind: Kind::Custom(settings_ui::about),
    },
];

pub fn entries(category: Category) -> impl Iterator<Item = &'static Entry> {
    ENTRIES.iter().filter(move |e| e.category == category)
}

/// Puts the values of one category (or all) back to their defaults.
pub fn reset(prefs: &mut Prefs, category: Option<Category>) {
    let default = Prefs::default();
    for e in ENTRIES
        .iter()
        .filter(|e| category.is_none_or(|c| c == e.category))
    {
        match e.kind {
            Kind::Toggle { get, set } => set(prefs, get(&default)),
            Kind::Choice { get, set, .. } => set(prefs, get(&default)),
            Kind::Seconds { get, set } => set(prefs, get(&default)),
            Kind::Custom(_) => {}
        }
    }
}

/// True if any value in `category` differs from its default.
pub fn is_changed(prefs: &Prefs, category: Category) -> bool {
    let default = Prefs::default();
    entries(category).any(|e| match e.kind {
        Kind::Toggle { get, .. } => get(prefs) != get(&default),
        Kind::Choice { get, .. } => get(prefs) != get(&default),
        Kind::Seconds { get, .. } => get(prefs) != get(&default),
        Kind::Custom(_) => false,
    })
}

/// Reads one `key=value` line of the preferences file into `prefs`; false
/// if the key is not a registry setting. Bad values are ignored.
pub fn read(prefs: &mut Prefs, key: &str, value: &str) -> bool {
    let Some(e) = ENTRIES.iter().find(|e| e.is_value() && e.id == key) else {
        return false;
    };
    match e.kind {
        Kind::Toggle { set, .. } => {
            if let Ok(v) = value.parse() {
                set(prefs, v);
            }
        }
        Kind::Choice {
            options,
            set,
            nearest,
            ..
        } => {
            let exact = options.iter().position(|o| o.key == value);
            let close = || {
                let v: f64 = value.parse().ok()?;
                let distance = |o: &Opt| {
                    o.key
                        .parse::<f64>()
                        .map_or(f64::INFINITY, |k| (k - v).abs())
                };
                (0..options.len())
                    .min_by(|&a, &b| distance(&options[a]).total_cmp(&distance(&options[b])))
            };
            if let Some(i) = exact.or_else(|| if nearest { close() } else { None }) {
                set(prefs, i);
            }
        }
        Kind::Seconds { set, .. } => {
            if let Some(v) = value.parse::<f32>().ok().filter(|v| v.is_finite()) {
                set(prefs, speed::tidy(v));
            }
        }
        Kind::Custom(_) => {}
    }
    true
}

/// `key=value` lines for every registry setting.
pub fn write(prefs: &Prefs, out: &mut String) {
    for e in ENTRIES {
        match e.kind {
            Kind::Toggle { get, .. } => *out += &format!("{}={}\n", e.id, get(prefs)),
            Kind::Choice { options, get, .. } => {
                *out += &format!("{}={}\n", e.id, options[get(prefs)].key);
            }
            Kind::Seconds { get, .. } => {
                *out += &format!("{}={}\n", e.id, speed::tidy(get(prefs)));
            }
            Kind::Custom(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        for (i, e) in ENTRIES.iter().enumerate() {
            assert!(
                ENTRIES[..i].iter().all(|o| o.id != e.id),
                "duplicate id {}",
                e.id
            );
        }
        for (n, o) in GRID_SIZES.iter().zip(&GRID_OPTIONS) {
            assert_eq!(n.to_string(), o.key);
        }
        // Every page has something on it.
        for c in Category::ALL {
            assert!(entries(c).next().is_some(), "{c:?} is empty");
            assert_eq!(Category::from_key(&c.label().replace(' ', "")), Some(c));
        }
    }

    /// A `setting.<id>` text in en.txt is the registry's own English, which
    /// the search uses (see `settings_ui`).
    #[test]
    fn translated_labels_match_the_registry() {
        crate::i18n::set_language("en");
        for e in ENTRIES {
            let label = crate::i18n::get(&format!("setting.{}", e.id));
            assert!(label.is_none_or(|l| l == e.label), "{}", e.id);
            let description = crate::i18n::get(&format!("setting.{}.description", e.id));
            assert!(description.is_none_or(|d| d == e.description), "{}", e.id);
        }
    }

    #[test]
    fn search_matches_all_words() {
        let ids = |q: &str| -> Vec<&str> {
            ENTRIES
                .iter()
                .filter(|e| e.matches(q))
                .map(|e| e.id)
                .collect()
        };
        assert_eq!(ids("dark mode"), ["theme"]);
        assert!(ids("MPCORB download").contains(&"mpcorb"));
        assert!(ids("Blink").contains(&"blink_seconds"));
        assert!(ids("").is_empty());
        assert!(ids("zebra").is_empty());
    }

    #[test]
    fn reset_one_section() {
        let mut p = Prefs {
            theme: ThemeChoice::Dark,
            invert: true,
            grid_size: 8,
            ..Prefs::default()
        };
        assert!(is_changed(&p, Category::Viewing));
        reset(&mut p, Some(Category::Viewing));
        assert!(!p.invert && !is_changed(&p, Category::Viewing));
        assert_eq!((p.theme, p.grid_size), (ThemeChoice::Dark, 8));
        // A blink speed off the old list counts as changed and resets too.
        p.blink_seconds = 2.35;
        assert!(is_changed(&p, Category::Viewing));
        reset(&mut p, Some(Category::Viewing));
        assert_eq!(p.blink_seconds, Prefs::default().blink_seconds);
        reset(&mut p, None);
        assert_eq!(p, Prefs::default());
    }
}
