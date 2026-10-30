//! Search history: what each student searched, marked and reported, and what
//! became of it (preliminary detection, provisional designation, number).
//!
//! Every search is one small JSON file in the history folder, named after its
//! id. One file per search means a damaged file loses one entry, files can
//! be copied or backed up by hand, and a shared folder (OneDrive, Google
//! Drive, a network drive) collects a whole class's searches without two
//! computers ever writing the same file. Teachers without a shared folder use
//! export and import, which merge by id and keep the newer copy.

use std::path::{Path, PathBuf};

use crate::format;
use crate::json::{self, Value};
use crate::prefs;

/// Format names written into the files, so other tools can recognise them.
const RECORD_FORMAT: &str = "openceres-search";
const EXPORT_FORMAT: &str = "openceres-history";

/// Where a search stands. The later steps follow IASC's process: a reported
/// detection becomes a preliminary detection when the MPC accepts it, then
/// gets a provisional designation, and finally a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Searching,
    NothingFound,
    Reported,
    Preliminary,
    Provisional,
    Numbered,
    NotConfirmed,
}

impl Status {
    pub const ALL: [Status; 7] = [
        Status::Searching,
        Status::NothingFound,
        Status::Reported,
        Status::Preliminary,
        Status::Provisional,
        Status::Numbered,
        Status::NotConfirmed,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Status::Searching => "searching",
            Status::NothingFound => "nothing_found",
            Status::Reported => "reported",
            Status::Preliminary => "preliminary",
            Status::Provisional => "provisional",
            Status::Numbered => "numbered",
            Status::NotConfirmed => "not_confirmed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Searching => "Searching",
            Status::NothingFound => "Nothing found",
            Status::Reported => "Reported",
            Status::Preliminary => "Preliminary detection",
            Status::Provisional => "Provisional designation",
            Status::Numbered => "Numbered",
            Status::NotConfirmed => "Not confirmed",
        }
    }

    fn from_key(key: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.key() == key)
    }

    /// Accepted by the MPC: the searches students are proudest of.
    pub fn is_find(self) -> bool {
        matches!(
            self,
            Status::Preliminary | Status::Provisional | Status::Numbered
        )
    }

    /// The report has been sent (whatever came of it).
    pub fn is_reported(self) -> bool {
        self.is_find() || matches!(self, Status::Reported | Status::NotConfirmed)
    }

    /// Statuses that come with a designation worth writing down.
    pub fn has_designation(self) -> bool {
        self.is_find()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Record {
    /// Unique and file-name safe: `<set>-<created>-<random>`.
    pub id: String,
    /// Set name used in reports, e.g. `XY57_p01`.
    pub set: String,
    /// DATE-OBS of the first frame, to tell apart sets that share a name.
    pub observed: Option<String>,
    /// MPC observatory code of the images, e.g. `F51`.
    pub observatory: Option<String>,
    pub searched_by: String,
    /// Unix times.
    pub created: u64,
    pub updated: u64,
    pub grid: usize,
    /// Marked boxes as (row, column), 1-based from the top-left.
    pub boxes: Vec<(usize, usize)>,
    pub status: Status,
    /// e.g. a team's preliminary designation or `2026 UX12`.
    pub designation: String,
    pub notes: String,
    /// Known objects in the field, e.g. `133105 2003 OG10`.
    pub known: Vec<String>,
    /// The files that were opened, so the search can be opened again.
    pub files: Vec<String>,
}

impl Record {
    pub fn new(set: &str, searched_by: &str, grid: usize) -> Record {
        let now = format::unix_now();
        Record {
            id: new_id(set, now),
            set: set.to_owned(),
            searched_by: searched_by.trim().to_owned(),
            created: now,
            updated: now,
            grid,
            ..Record::default()
        }
    }

    pub fn report(&self) -> String {
        report_line(&self.set, &self.boxes)
    }

    /// Whether this is the same image set: same name and, when both are
    /// known, the same observation time.
    pub fn is_set(&self, set: &str, observed: Option<&str>) -> bool {
        self.set == set
            && match (self.observed.as_deref(), observed) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }

    pub fn is_by(&self, name: &str) -> bool {
        same_name(&self.searched_by, name)
    }

    /// Only a search someone has worked on is kept: opening a set alone
    /// does not add to the history.
    pub fn has_content(&self) -> bool {
        !self.boxes.is_empty()
            || self.status != Status::Searching
            || !self.notes.trim().is_empty()
            || !self.designation.trim().is_empty()
    }

    fn file_name(&self) -> String {
        format!("{}.json", self.id)
    }

    pub fn to_json(&self, indent: &str) -> String {
        let boxes: Vec<String> = self.boxes.iter().map(|&b| box_label(b)).collect();
        let opt = |v: &Option<String>| v.as_deref().map_or_else(|| "null".into(), json::quote);
        let fields = [
            ("format", json::quote(RECORD_FORMAT)),
            ("version", "1".into()),
            ("id", json::quote(&self.id)),
            ("set", json::quote(&self.set)),
            ("observed", opt(&self.observed)),
            ("observatory", opt(&self.observatory)),
            ("searched_by", json::quote(&self.searched_by)),
            ("created", self.created.to_string()),
            ("updated", self.updated.to_string()),
            ("grid", self.grid.to_string()),
            ("boxes", json::string_array(&boxes)),
            ("report", json::quote(&self.report())),
            ("status", json::quote(self.status.key())),
            ("designation", json::quote(&self.designation)),
            ("notes", json::quote(&self.notes)),
            ("known", json::string_array(&self.known)),
            ("files", json::string_array(&self.files)),
            ("app", json::quote(crate::update::CURRENT_VERSION)),
        ];
        let body: Vec<String> = fields
            .iter()
            .map(|(k, v)| format!("{indent}  {}: {v}", json::quote(k)))
            .collect();
        format!("{{\n{}\n{indent}}}", body.join(",\n"))
    }

    /// Reads a record; `None` if it has no usable id or set name. Unknown
    /// fields are ignored and missing ones take their defaults, so files
    /// from other versions still load.
    pub fn from_value(v: &Value) -> Option<Record> {
        let text = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| {
            v.get(key)
                .and_then(Value::as_f64)
                .filter(|n| *n >= 0.0)
                .map(|n| n as u64)
        };
        let list = |key: &str| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let id = text("id").filter(|id| is_safe_id(id))?;
        let set = text("set").filter(|s| !s.trim().is_empty())?;
        let grid = number("grid").map_or(4, |g| g as usize).clamp(1, 12);
        let boxes = list("boxes")
            .iter()
            .filter_map(|b| parse_box(b))
            .filter(|&(r, c)| r <= grid && c <= grid)
            .collect();
        let created = number("created").unwrap_or(0);
        Some(Record {
            id,
            set,
            observed: text("observed"),
            observatory: text("observatory"),
            searched_by: text("searched_by").unwrap_or_default(),
            created,
            updated: number("updated").unwrap_or(created),
            grid,
            boxes,
            status: text("status")
                .and_then(|s| Status::from_key(&s))
                .unwrap_or_default(),
            designation: text("designation").unwrap_or_default(),
            notes: text("notes").unwrap_or_default(),
            known: list("known"),
            files: list("files"),
        })
    }
}

/// The team's box name: row × column, e.g. `1x2` = top row, second column.
pub fn box_label((row, col): (usize, usize)) -> String {
    format!("{row}x{col}")
}

fn parse_box(label: &str) -> Option<(usize, usize)> {
    let (r, c) = label.split_once('x')?;
    let (r, c) = (r.trim().parse().ok()?, c.trim().parse().ok()?);
    (r >= 1 && c >= 1).then_some((r, c))
}

/// e.g. `XY57_p01: 1x2 2x1`, boxes in reading order (by row, then column).
pub fn report_line(name: &str, marks: &[(usize, usize)]) -> String {
    let mut boxes = marks.to_vec();
    boxes.sort_unstable();
    let boxes: Vec<String> = boxes.into_iter().map(box_label).collect();
    format!("{name}: {}", boxes.join(" "))
}

/// Names match ignoring case and surrounding spaces.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Ids become file names, so imported ones must not reach outside the
/// history folder or hide as dot files.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn new_id(set: &str, now: u64) -> String {
    use std::hash::{BuildHasher, Hasher};
    let slug: String = set
        .chars()
        .take(60)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let slug = if slug.trim_matches('_').is_empty() {
        "set".to_owned()
    } else {
        slug
    };
    // RandomState is seeded randomly per process; mixing in the clock keeps
    // two searches started in the same second apart.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    format!("{slug}-{now}-{:06x}", h.finish() & 0xff_ffff)
}

// ---------------------------------------------------------------------------
// Settings: whose searches these are and where they are kept

/// The student's name and the history folder, kept in `history.txt` in the
/// data folder (next to the preferences, which hold only simple values).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// Default "Searched by" for new searches.
    pub name: String,
    /// A chosen folder, e.g. a shared one; `None` = the data folder's.
    pub folder: Option<PathBuf>,
}

impl Config {
    pub fn load() -> Config {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Config::default, |t| Config::parse(&t))
    }

    fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("name", v)) => c.name = v.trim().to_owned(),
                Some(("folder", v)) if !v.trim().is_empty() => {
                    c.folder = Some(PathBuf::from(v.trim()));
                }
                _ => {}
            }
        }
        c
    }

    fn to_text(&self) -> String {
        let folder = self
            .folder
            .as_ref()
            .map_or_else(String::new, |f| f.display().to_string());
        format!(
            "name={}\nfolder={folder}\n",
            self.name.replace(['\n', '\r'], " ")
        )
    }

    /// Best effort, like the preferences.
    pub fn save(&self) {
        if let Some(path) = config_path() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, self.to_text());
        }
    }

    /// The folder in use: `OPENCERES_HISTORY_DIR` (tests and screenshots),
    /// else the chosen folder, else `History` in the data folder.
    pub fn dir(&self) -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("OPENCERES_HISTORY_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir));
        }
        self.folder.clone().or_else(default_dir)
    }
}

pub fn default_dir() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("History"))
}

fn config_path() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("history.txt"))
}

// ---------------------------------------------------------------------------
// Files

/// Every readable search in `dir`, most recently changed first, and how many
/// files could not be read. A missing folder is an empty history.
pub fn load_all(dir: &Path) -> Result<(Vec<Record>, usize), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(e) => return Err(format!("Couldn't read the history folder: {e}")),
    };
    let mut records = Vec::new();
    let mut unreadable = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let record = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| json::parse(&t).ok())
            .and_then(|v| Record::from_value(&v));
        match record {
            Some(r) => records.push(r),
            None => unreadable += 1,
        }
    }
    records.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
    Ok((records, unreadable))
}

/// Writes the record's file, replacing it in one step (write a hidden
/// temporary file, then rename) so a crash never leaves half a file.
pub fn save(dir: &Path, record: &Record) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("Couldn't save the search history: {e}");
    std::fs::create_dir_all(dir).map_err(fail)?;
    let path = dir.join(record.file_name());
    let tmp = dir.join(format!(".{}.tmp", record.file_name()));
    std::fs::write(&tmp, record.to_json("") + "\n").map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

pub fn delete(dir: &Path, record: &Record) -> Result<(), String> {
    match std::fs::remove_file(dir.join(record.file_name())) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Couldn't delete the search: {e}")),
    }
}

/// All `records` in one file, for a teacher to import.
pub fn export_json(records: &[Record]) -> String {
    let items: Vec<String> = records.iter().map(|r| r.to_json("    ")).collect();
    format!(
        "{{\n  \"format\": {},\n  \"version\": 1,\n  \"exported\": {},\n  \"records\": [\n    {}\n  ]\n}}\n",
        json::quote(EXPORT_FORMAT),
        format::unix_now(),
        items
            .iter()
            .map(|s| s.trim_start())
            .collect::<Vec<_>>()
            .join(",\n    ")
    )
}

/// A spreadsheet of `records`, one row each.
pub fn export_csv(records: &[Record]) -> String {
    let mut out = String::from(
        "Set,Observed (UTC),Observatory,Searched by,Status,Designation,Boxes,Report,\
         Known objects,Notes,Searched (UTC),Updated (UTC)\r\n",
    );
    for r in records {
        let boxes: Vec<String> = {
            let mut b = r.boxes.clone();
            b.sort_unstable();
            b.into_iter().map(box_label).collect()
        };
        let row = [
            r.set.clone(),
            r.observed.clone().unwrap_or_default(),
            r.observatory.clone().unwrap_or_default(),
            r.searched_by.clone(),
            r.status.label().to_owned(),
            r.designation.clone(),
            boxes.join(" "),
            r.report(),
            r.known.join("; "),
            r.notes.clone(),
            format::unix_utc(r.created),
            format::unix_utc(r.updated),
        ];
        let cells: Vec<String> = row.iter().map(|c| csv_cell(c)).collect();
        out += &cells.join(",");
        out += "\r\n";
    }
    out
}

fn csv_cell(text: &str) -> String {
    // A leading = + - @ would run as a formula in a spreadsheet.
    let text = if text.starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Imported {
    pub added: usize,
    pub updated: usize,
    /// Already in the history, as new or newer.
    pub unchanged: usize,
}

/// Merges an export (or a single search file) into `dir`. A search that is
/// already there is replaced only by a newer copy of itself.
pub fn import(dir: &Path, text: &str) -> Result<Imported, String> {
    let not_history = || "This file isn't an OpenCeres history export.".to_owned();
    let root = json::parse(text.trim_start_matches('\u{feff}')).map_err(|_| not_history())?;
    let incoming: Vec<Record> = match root.get("records").and_then(Value::as_array) {
        Some(items) => items.iter().filter_map(Record::from_value).collect(),
        None => Record::from_value(&root).into_iter().collect(),
    };
    if incoming.is_empty() {
        return Err(not_history());
    }
    let (existing, _) = load_all(dir)?;
    let mut result = Imported::default();
    for r in incoming {
        match existing.iter().find(|e| e.id == r.id) {
            Some(e) if e.updated >= r.updated => result.unchanged += 1,
            Some(_) => {
                save(dir, &r)?;
                result.updated += 1;
            }
            None => {
                save(dir, &r)?;
                result.added += 1;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        Record {
            id: "XY57_p01-1793349698-0a1b2c".into(),
            set: "XY57_p01".into(),
            observed: Some("2026-10-04T10:15:39.123".into()),
            observatory: Some("F51".into()),
            searched_by: "Ayşe Yılmaz".into(),
            created: 1_793_349_698,
            updated: 1_793_349_800,
            grid: 4,
            boxes: vec![(2, 3), (1, 4)],
            status: Status::Provisional,
            designation: "2026 UX12".into(),
            notes: "Faint, \"moving\" fast\nsecond line".into(),
            known: vec!["133105 2003 OG10".into()],
            files: vec!["/Users/me/Downloads/XY57_p01.zip".into()],
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openceres-history-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn record_round_trips() {
        let r = sample();
        let back = Record::from_value(&json::parse(&r.to_json("")).unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(r.report(), "XY57_p01: 1x4 2x3");
    }

    #[test]
    fn reads_partial_and_rejects_unsafe_records() {
        let r = Record::from_value(&json::parse(r#"{"id":"a-1","set":"S1"}"#).unwrap()).unwrap();
        assert_eq!((r.grid, r.status, r.boxes.len()), (4, Status::Searching, 0));
        for bad in [
            r#"{"id":"../../evil","set":"S"}"#,
            r#"{"id":".hidden","set":"S"}"#,
            r#"{"id":"a","set":" "}"#,
            r#"{"set":"S"}"#,
        ] {
            assert_eq!(
                Record::from_value(&json::parse(bad).unwrap()),
                None,
                "{bad}"
            );
        }
        // Boxes outside the grid are dropped.
        let r = Record::from_value(
            &json::parse(r#"{"id":"a","set":"S","grid":3,"boxes":["1x1","4x1","x"]}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(r.boxes, [(1, 1)]);
    }

    #[test]
    fn new_ids_are_safe_and_distinct() {
        let a = Record::new("XY57 p01/../x", "A", 4);
        let b = Record::new("XY57 p01/../x", "A", 4);
        assert!(is_safe_id(&a.id), "{}", a.id);
        assert_ne!(a.id, b.id);
        assert!(is_safe_id(&Record::new("", "A", 4).id));
    }

    #[test]
    fn saves_loads_and_deletes() {
        let dir = temp_dir("files");
        assert_eq!(load_all(&dir).unwrap().0, []); // missing folder = empty
        let mut older = sample();
        older.id = "older-1".into();
        older.updated = 10;
        save(&dir, &sample()).unwrap();
        save(&dir, &older).unwrap();
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a record").unwrap();
        let (records, unreadable) = load_all(&dir).unwrap();
        assert_eq!(unreadable, 1);
        assert_eq!(records, [sample(), older.clone()]); // newest first
        delete(&dir, &older).unwrap();
        assert_eq!(load_all(&dir).unwrap().0, [sample()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_merges_by_id_keeping_newer() {
        let dir = temp_dir("import");
        let mut mine = sample();
        save(&dir, &mine).unwrap();
        let mut theirs = sample();
        theirs.id = "XY57_p01-1793349999-ffffff".into();
        theirs.searched_by = "Mehmet".into();
        let mut stale = sample();
        stale.updated -= 100;
        stale.notes = "old".into();
        let text = export_json(&[stale, theirs.clone()]);
        let got = import(&dir, &text).unwrap();
        assert_eq!(
            got,
            Imported {
                added: 1,
                updated: 0,
                unchanged: 1
            }
        );
        mine.updated += 50;
        mine.status = Status::Numbered;
        let got = import(&dir, &mine.to_json("")).unwrap(); // a single record file
        assert_eq!(got.updated, 1);
        let (records, _) = load_all(&dir).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], mine);
        assert!(import(&dir, "{\"hello\": 1}").is_err());
        assert!(import(&dir, "nonsense").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let csv = export_csv(&[sample()]);
        let mut lines = csv.split("\r\n");
        assert!(lines.next().unwrap().starts_with("Set,Observed"));
        let row = lines.next().unwrap();
        assert!(row.starts_with("XY57_p01,2026-10-04T10:15:39.123,F51,Ayşe Yılmaz,"));
        assert!(row.contains("\"Faint, \"\"moving\"\" fast\nsecond"));
        assert_eq!(csv_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_cell("-"), "'-");
    }

    #[test]
    fn config_round_trips() {
        let c = Config {
            name: "Ayşe".into(),
            folder: Some(PathBuf::from("/Volumes/Shared/Class 7B")),
        };
        assert_eq!(Config::parse(&c.to_text()), c);
        assert_eq!(Config::parse("junk\nname=\nfolder=\n"), Config::default());
    }

    #[test]
    fn matching_sets_and_names() {
        let r = sample();
        assert!(r.is_set("XY57_p01", Some("2026-10-04T10:15:39.123")));
        assert!(r.is_set("XY57_p01", None));
        assert!(!r.is_set("XY57_p01", Some("2026-10-05T00:00:00")));
        assert!(!r.is_set("XY57_p02", None));
        assert!(r.is_by("  ayşe yılmaz "));
        assert!(!Record::new("S", "A", 4).has_content());
    }
}
