//! Preferences that survive restarts, in a small `key=value` text file in the
//! platform's config folder. Unknown keys and bad values are ignored, so an
//! old or hand-edited file never stops the app from starting.
//!
//! Most values are *defaults*: what a new session starts with. Toggling stars
//! or invert in the toolbar changes only the current session; the Settings
//! window changes the defaults (and applies them right away). Which keys
//! exist is defined by the settings registry in [`crate::settings`].

use std::path::PathBuf;

use crate::settings;

/// Blink speeds, seconds per frame.
pub const SPEEDS: [f32; 8] = [0.2, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0];
/// Report grid sizes (n × n boxes).
pub const GRID_SIZES: [usize; 5] = [3, 4, 5, 6, 8];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
}

/// Where known objects come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KnownSource {
    /// IMCCE's SkyBoT web service.
    #[default]
    SkyBot,
    /// The MPC's MPCORB catalogue, downloaded into the data folder.
    Mpcorb,
}

impl KnownSource {
    pub const ALL: [KnownSource; 2] = [KnownSource::SkyBot, KnownSource::Mpcorb];

    pub fn short(self) -> &'static str {
        match self {
            KnownSource::SkyBot => "SkyBoT",
            KnownSource::Mpcorb => "MPCORB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prefs {
    pub theme: ThemeChoice,
    pub blink_seconds: f32,
    pub show_stars: bool,
    pub invert: bool,
    pub grid_size: usize,
    /// Whether the report side panel is open.
    pub panel_open: bool,
    pub known_source: KnownSource,
    pub update_check: bool,
    /// Unix time of the last update check that found nothing new (0 = never).
    pub update_last_check: u64,
    /// Unix time of the last background MPCORB check that found the
    /// catalogue current (0 = never).
    pub mpcorb_last_check: u64,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::System,
            blink_seconds: 0.5,
            show_stars: false,
            invert: false,
            grid_size: 4,
            panel_open: false,
            known_source: KnownSource::SkyBot,
            update_check: true,
            update_last_check: 0,
            mpcorb_last_check: 0,
        }
    }
}

impl Prefs {
    /// Settings from the registry (see [`crate::settings`]), plus the
    /// app's own state.
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if settings::read(&mut p, key, value) {
                continue;
            }
            match key {
                "panel_open" => p.panel_open = value.parse().unwrap_or(p.panel_open),
                "update_last_check" => {
                    p.update_last_check = value.parse().unwrap_or(p.update_last_check);
                }
                "mpcorb_last_check" => {
                    p.mpcorb_last_check = value.parse().unwrap_or(p.mpcorb_last_check);
                }
                _ => {}
            }
        }
        p
    }

    pub fn to_text(self) -> String {
        let mut out = String::new();
        settings::write(&self, &mut out);
        out += &format!(
            "panel_open={}\nupdate_last_check={}\nmpcorb_last_check={}\n",
            self.panel_open, self.update_last_check, self.mpcorb_last_check
        );
        out
    }

    pub fn load() -> Prefs {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Prefs::default, |t| Prefs::parse(&t))
    }

    /// Best effort: preferences are a convenience, so failures are ignored.
    pub fn save(self) {
        if let Some(path) = path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, self.to_text());
        }
    }
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The app's own folder for preferences and the offline catalogue, e.g.
/// `~/Library/Application Support/OpenCeres` on macOS.
pub fn data_dir() -> Option<PathBuf> {
    Some(if cfg!(target_os = "macos") {
        env_dir("HOME")?.join("Library/Application Support/OpenCeres")
    } else if cfg!(windows) {
        env_dir("APPDATA")?.join("OpenCeres")
    } else {
        env_dir("XDG_CONFIG_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".config")))?
            .join("openceres")
    })
}

fn path() -> Option<PathBuf> {
    data_dir().map(|d| d.join("prefs.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let p = Prefs {
            theme: ThemeChoice::Dark,
            blink_seconds: 0.75,
            show_stars: true,
            invert: true,
            grid_size: 6,
            panel_open: true,
            known_source: KnownSource::Mpcorb,
            update_check: false,
            update_last_check: 1_793_279_073,
            mpcorb_last_check: 1_793_279_000,
        };
        assert_eq!(Prefs::parse(&p.to_text()), p);
    }

    #[test]
    fn ignores_junk_and_bad_values() {
        let p = Prefs::parse(
            "# note\ninvert = yes\ngrid_size=7\nblink_seconds=0.31\ntheme=blue\nfoo=bar\n=\n",
        );
        assert_eq!(
            p,
            Prefs {
                blink_seconds: 0.3,
                ..Prefs::default()
            }
        );
    }

    #[test]
    fn reads_files_from_older_versions() {
        // `grid_on` used to be saved; it is now a per-session toggle.
        let p = Prefs::parse("invert=true\nblink_seconds=1\ngrid_on=true\ngrid_size=5\n");
        assert!(p.invert);
        assert_eq!((p.blink_seconds, p.grid_size), (1.0, 5));
        assert!(!p.panel_open);
    }
}
