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
    /// MPCORB once it is downloaded, SkyBoT until then: the offline
    /// catalogue answers in a second and works without the internet.
    #[default]
    Auto,
    /// IMCCE's SkyBoT web service.
    SkyBot,
    /// The MPC's MPCORB catalogue, downloaded into the data folder.
    Mpcorb,
}

impl KnownSource {
    pub const ALL: [KnownSource; 3] = [KnownSource::Auto, KnownSource::SkyBot, KnownSource::Mpcorb];

    pub fn short(self) -> &'static str {
        match self {
            KnownSource::Auto => "Automatic",
            KnownSource::SkyBot => "SkyBoT",
            KnownSource::Mpcorb => "MPCORB",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prefs {
    pub theme: ThemeChoice,
    /// Seconds per frame, [`crate::speed::MIN`] to [`crate::speed::MAX`].
    pub blink_seconds: f32,
    pub show_stars: bool,
    pub invert: bool,
    /// Hold the stars still in the blink by aligning frames on them.
    pub align: bool,
    /// The box report (click grid boxes where something moves), the older
    /// way to report. Off unless turned on in Settings → Features.
    pub box_report: bool,
    pub show_grid: bool,
    pub grid_size: usize,
    /// Whether the report side panel is open.
    pub panel_open: bool,
    /// The panel has been opened for a loaded set once (it opens by
    /// itself the first time, then stays as the user leaves it).
    pub panel_shown: bool,
    pub show_known: bool,
    pub known_source: KnownSource,
    /// VizieR mirror for Gaia stars: 0 = automatic, else 1 + index into
    /// [`openceres_core::gaia::MIRRORS`].
    pub gaia_mirror: usize,
    /// The mirror that last answered (automatic mode tries it first).
    pub gaia_last_good: usize,
    pub update_check: bool,
    /// Unix time of the last update check that found nothing new (0 = never).
    pub update_last_check: u64,
    /// UI language, a code from [`crate::i18n::LANGUAGES`].
    pub language: &'static str,
    /// The welcome window has been seen (finished or skipped).
    pub onboarded: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeChoice::System,
            blink_seconds: 0.5,
            show_stars: false,
            invert: false,
            align: true,
            box_report: false,
            show_grid: true,
            grid_size: 4,
            panel_open: false,
            panel_shown: false,
            show_known: true,
            known_source: KnownSource::Auto,
            gaia_mirror: 0,
            gaia_last_good: 0,
            update_check: true,
            update_last_check: 0,
            language: "en",
            onboarded: false,
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
                "panel_shown" => p.panel_shown = value.parse().unwrap_or(p.panel_shown),
                "gaia_last_good" => p.gaia_last_good = value.parse().unwrap_or(p.gaia_last_good),
                "update_last_check" => {
                    p.update_last_check = value.parse().unwrap_or(p.update_last_check);
                }
                "language" => {
                    if let Some(l) = crate::i18n::find(value) {
                        p.language = l.code;
                    }
                }
                "onboarded" => p.onboarded = value.parse().unwrap_or(p.onboarded),
                _ => {}
            }
        }
        p
    }

    pub fn to_text(self) -> String {
        let mut out = String::new();
        settings::write(&self, &mut out);
        out += &format!(
            "panel_open={}
panel_shown={}
gaia_last_good={}
update_last_check={}
language={}
onboarded={}
",
            self.panel_open,
            self.panel_shown,
            self.gaia_last_good,
            self.update_last_check,
            self.language,
            self.onboarded
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
            blink_seconds: 1.35,
            show_stars: true,
            invert: true,
            align: false,
            box_report: true,
            show_grid: false,
            grid_size: 6,
            panel_open: true,
            panel_shown: true,
            show_known: false,
            known_source: KnownSource::Mpcorb,
            gaia_mirror: 2,
            gaia_last_good: 1,
            update_check: false,
            update_last_check: 1_793_279_073,
            language: "en",
            onboarded: true,
        };
        assert_eq!(Prefs::parse(&p.to_text()), p);
    }

    #[test]
    fn ignores_junk_and_bad_values() {
        let p = Prefs::parse(
            "# note\ninvert = yes\ngrid_size=7\nblink_seconds=fast\ntheme=blue\nfoo=bar\n=\n\
             language=xx\nonboarded=maybe\n",
        );
        assert_eq!(p, Prefs::default());
        assert_eq!(Prefs::parse("blink_seconds=NaN\n"), Prefs::default());
        assert_eq!(Prefs::parse("blink_seconds=inf\n"), Prefs::default());
    }

    #[test]
    fn reads_files_from_older_versions() {
        // `grid_on` used to be saved; it is now a per-session toggle.
        let p = Prefs::parse("invert=true\nblink_seconds=1\ngrid_on=true\ngrid_size=5\n");
        assert!(p.invert);
        assert_eq!((p.blink_seconds, p.grid_size), (1.0, 5));
        assert!(!p.panel_open);
        // Every speed the old fixed list could save still loads as itself.
        for (text, secs) in [
            ("0.2", 0.2),
            ("0.3", 0.3),
            ("0.5", 0.5),
            ("0.75", 0.75),
            ("1", 1.0),
            ("1.5", 1.5),
            ("2", 2.0),
            ("3", 3.0),
        ] {
            let p = Prefs::parse(&format!("blink_seconds={text}\n"));
            assert_eq!(p.blink_seconds, secs, "{text}");
            assert_eq!(Prefs::parse(&p.to_text()), p);
        }
    }

    #[test]
    fn blink_speed_is_continuous_and_clamped() {
        for (text, secs) in [
            ("0.31", 0.31),
            ("2.35", 2.35),
            ("0.333333", 0.33),
            ("0.01", 0.1),
            ("9", 5.0),
            (" 4.2 ", 4.2),
        ] {
            let p = Prefs::parse(&format!("blink_seconds={text}\n"));
            assert_eq!(p.blink_seconds, secs, "{text}");
            assert_eq!(Prefs::parse(&p.to_text()), p, "{text}");
        }
    }
}
