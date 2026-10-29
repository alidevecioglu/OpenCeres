//! Preferences that survive restarts, in a small `key=value` text file in the
//! platform's config folder. Unknown keys and bad values are ignored, so an
//! old or hand-edited file never stops the app from starting.
//!
//! Most values are *defaults*: what a new session starts with. Toggling stars
//! or invert in the toolbar changes only the current session; the Settings
//! window changes the defaults (and applies them right away).

use std::path::PathBuf;

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

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
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
        }
    }
}

impl Prefs {
    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let flag = |old: bool| value.parse().unwrap_or(old);
            match key.trim() {
                "theme" => {
                    if let Some(t) = ThemeChoice::ALL.into_iter().find(|t| t.key() == value) {
                        p.theme = t;
                    }
                }
                "blink_seconds" => {
                    if let Ok(s) = value.parse::<f32>() {
                        p.blink_seconds = nearest_speed(s);
                    }
                }
                "show_stars" => p.show_stars = flag(p.show_stars),
                "invert" => p.invert = flag(p.invert),
                "grid_size" => {
                    if let Some(n) = value.parse().ok().filter(|n| GRID_SIZES.contains(n)) {
                        p.grid_size = n;
                    }
                }
                "panel_open" => p.panel_open = flag(p.panel_open),
                _ => {}
            }
        }
        p
    }

    pub fn to_text(self) -> String {
        format!(
            "theme={}\nblink_seconds={}\nshow_stars={}\ninvert={}\ngrid_size={}\npanel_open={}\n",
            self.theme.key(),
            self.blink_seconds,
            self.show_stars,
            self.invert,
            self.grid_size,
            self.panel_open
        )
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

pub fn nearest_speed(s: f32) -> f32 {
    SPEEDS
        .into_iter()
        .min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs()))
        .unwrap_or(0.5)
}

fn env_dir(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn path() -> Option<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        env_dir("HOME")?.join("Library/Application Support/OpenCeres")
    } else if cfg!(windows) {
        env_dir("APPDATA")?.join("OpenCeres")
    } else {
        env_dir("XDG_CONFIG_HOME")
            .or_else(|| env_dir("HOME").map(|h| h.join(".config")))?
            .join("openceres")
    };
    Some(dir.join("prefs.txt"))
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
