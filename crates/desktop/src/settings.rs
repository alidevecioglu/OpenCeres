//! The settings registry: every setting is declared once, in [`ENTRIES`].
//! The Settings window draws its pages from it, the search box filters it,
//! the preferences file is read and written through it, and "Reset this
//! section" / "Reset all" put its values back to [`Prefs::default`].
//!
//! Adding a setting: add a field to [`Prefs`] (and its default), then one
//! [`Entry`] here. A `Toggle` or `Choice` is saved under its `id`; a
//! `Custom` entry draws itself (status rows, buttons, tables) and is not
//! saved by the registry.

use eframe::egui;

use crate::app::{App, history_ui, settings_ui};
use crate::icons::Icon;
use crate::prefs::{GRID_SIZES, KnownSource, Prefs, SPEEDS, ThemeChoice};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    General,
    Appearance,
    Viewing,
    ReportGrid,
    Known,
    History,
    Updates,
    Shortcuts,
    About,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::General,
        Category::Appearance,
        Category::Viewing,
        Category::ReportGrid,
        Category::Known,
        Category::History,
        Category::Updates,
        Category::Shortcuts,
        Category::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Appearance => "Appearance",
            Category::Viewing => "Viewing",
            Category::ReportGrid => "Report grid",
            Category::Known => "Known objects",
            Category::History => "History",
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
            Category::ReportGrid => Icon::Grid,
            Category::Known => Icon::Known,
            Category::History => Icon::History,
            Category::Updates => Icon::Update,
            Category::Shortcuts => Icon::Keyboard,
            Category::About => Icon::Info,
        }
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
const SPEED_OPTIONS: [Opt; 8] = [
    opt("0.2", "0.2 s per frame"),
    opt("0.3", "0.3 s per frame"),
    opt("0.5", "0.5 s per frame"),
    opt("0.75", "0.75 s per frame"),
    opt("1", "1 s per frame"),
    opt("1.5", "1.5 s per frame"),
    opt("2", "2 s per frame"),
    opt("3", "3 s per frame"),
];
const GRID_OPTIONS: [Opt; 5] = [
    opt("3", "3×3"),
    opt("4", "4×4"),
    opt("5", "5×5"),
    opt("6", "6×6"),
    opt("8", "8×8"),
];
const SOURCES: [Opt; 2] = [opt("skybot", "SkyBoT"), opt("mpcorb", "MPCORB")];

fn index_of<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

pub static ENTRIES: &[Entry] = &[
    // General
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
        description: "How long each frame is shown. , and . change it for now.",
        keywords: "speed seconds frame rate interval slower faster animation",
        kind: Kind::Choice {
            options: &SPEED_OPTIONS,
            get: |p| index_of(&SPEEDS, &p.blink_seconds),
            set: |p, i| p.blink_seconds = SPEEDS[i],
            nearest: true,
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
    // Report grid
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
        id: "known_source",
        category: Category::Known,
        label: "Source",
        description: "SkyBoT asks IMCCE's service online. MPCORB searches the Minor \
                      Planet Center's catalogue on this computer once downloaded.",
        keywords: "skybot mpcorb online offline asteroids comets lookup imcce mpc",
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
        description: "About 94 MB from the Minor Planet Center. Check for update asks \
                      the server without downloading.",
        keywords: "download update catalogue catalog offline orbit database mpc check",
        kind: Kind::Custom(settings_ui::mpcorb),
    },
    // History
    Entry {
        id: "history_name",
        category: Category::History,
        label: "Your name",
        description: "Filled in as “Searched by” on new searches. On a shared computer, \
                      each student can type their own name in the side panel instead.",
        keywords: "student name searched by observer person profile",
        kind: Kind::Custom(history_ui::name_setting),
    },
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
    // Updates
    Entry {
        id: "update_check",
        category: Category::Updates,
        label: "Check for updates automatically",
        description: "Once a day at most, asks GitHub whether a newer OpenCeres is out. \
                      Nothing is downloaded without asking.",
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
        for (s, o) in SPEEDS.iter().zip(&SPEED_OPTIONS) {
            assert_eq!(s.to_string(), o.key);
        }
        assert_eq!(SPEEDS.len(), SPEED_OPTIONS.len());
        for (n, o) in GRID_SIZES.iter().zip(&GRID_OPTIONS) {
            assert_eq!(n.to_string(), o.key);
        }
        // Every page has something on it.
        for c in Category::ALL {
            assert!(entries(c).next().is_some(), "{c:?} is empty");
            assert_eq!(Category::from_key(&c.label().replace(' ', "")), Some(c));
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
        reset(&mut p, None);
        assert_eq!(p, Prefs::default());
    }
}
