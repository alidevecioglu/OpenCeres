//! Update check against the project's GitHub releases.
//!
//! Uses the system `curl` (like the SkyBoT lookup) so no HTTP/TLS crate is
//! needed. Downloads are verified against the SHA-256 digest GitHub
//! publishes for every release asset before they are handed to the user.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json;

const LATEST_RELEASE: &str = "https://api.github.com/repos/alidevecioglu/OpenCeres/releases/latest";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// e.g. `0.2.0` (tag without the leading `v`).
    pub version: String,
    /// Release page, for "What's new".
    pub page: String,
    /// The download for this platform, if the release has one.
    pub asset: Option<Asset>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex SHA-256, from GitHub's `digest` field.
    pub sha256: Option<String>,
}

/// Outcome of a check, for the UI.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    UpToDate,
    Available(Release),
}

/// Asks GitHub for the latest release. Errors are short, user-readable
/// strings; a private or missing repository reads as "no releases".
pub fn check() -> Result<Check, String> {
    let output = crate::known::curl()
        .args(["-sS", "--max-time", "20", "-w", "\n%{http_code}"])
        .args(["-H", "Accept: application/vnd.github+json"])
        .args(["-H", &format!("User-Agent: OpenCeres/{CURRENT_VERSION}")])
        .arg(LATEST_RELEASE)
        .output()
        .map_err(|_| "Couldn't run curl to check for updates.".to_string())?;
    if !output.status.success() {
        return Err("Couldn't reach GitHub — check the internet connection.".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, status) = text.rsplit_once('\n').unwrap_or((&text, ""));
    match status.trim() {
        "200" => {}
        // Private repository or no release published yet.
        "404" => return Ok(Check::UpToDate),
        "403" | "429" => {
            return Err("GitHub is rate-limiting update checks; try again later.".into());
        }
        code => return Err(format!("GitHub answered {code} to the update check.")),
    }
    let release = parse_release(body, platform_suffix())?;
    Ok(if is_newer(&release.version, CURRENT_VERSION) {
        Check::Available(release)
    } else {
        Check::UpToDate
    })
}

/// File-name ending of this platform's download, as produced by the
/// release workflow (`.github/workflows/build.yml`).
pub fn platform_suffix() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("-macos.dmg")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("-windows-x86_64.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("-linux-x86_64.tar.gz")
    } else {
        None
    }
}

fn parse_release(json: &str, suffix: Option<&str>) -> Result<Release, String> {
    let bad = || "GitHub sent an unexpected update reply.".to_string();
    let root = json::parse(json).map_err(|_| bad())?;
    let tag = root
        .get("tag_name")
        .and_then(json::Value::as_str)
        .ok_or_else(bad)?;
    let page = root
        .get("html_url")
        .and_then(json::Value::as_str)
        .unwrap_or_default();
    let asset = suffix.and_then(|suffix| {
        root.get("assets")?.as_array()?.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            name.ends_with(suffix).then(|| Asset {
                name: name.to_string(),
                url: a
                    .get("browser_download_url")
                    .and_then(json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                size: a.get("size").and_then(json::Value::as_f64).unwrap_or(0.0) as u64,
                sha256: a
                    .get("digest")
                    .and_then(json::Value::as_str)
                    .and_then(|d| d.strip_prefix("sha256:"))
                    .map(str::to_ascii_lowercase),
            })
        })
    });
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: page.to_string(),
        asset,
    })
}

/// True if `candidate` is a newer `major.minor.patch` than `current`.
/// Pre-release tags (`0.2.0-beta`) never count as newer than a release.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> Option<[u64; 3]> {
        if v.contains('-') {
            return None;
        }
        let mut parts = v.split('.').map(|p| p.parse::<u64>().ok());
        let version = [
            parts.next()??,
            parts.next().unwrap_or(Some(0))?,
            parts.next().unwrap_or(Some(0))?,
        ];
        parts.next().is_none().then_some(version)
    }
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

/// Downloads `asset` into `dir` (normally the user's Downloads folder),
/// verifies its SHA-256 and returns the final path. A failed or mismatching
/// download leaves nothing behind.
pub fn download(asset: &Asset, dir: &Path) -> Result<PathBuf, String> {
    let target = dir.join(&asset.name);
    let partial = dir.join(format!("{}.part", asset.name));
    let status = crate::known::curl()
        .args(["-sSfL", "--max-time", "900", "-o"])
        .arg(&partial)
        .arg(&asset.url)
        .status()
        .map_err(|_| "Couldn't run curl to download the update.".to_string())?;
    // Removes the partial file and passes the message on.
    let fail = |msg: String| {
        let _ = std::fs::remove_file(&partial);
        msg
    };
    if !status.success() {
        return Err(fail(
            "The download failed — check the internet connection.".into(),
        ));
    }
    if let Some(expected) = &asset.sha256 {
        match sha256_file(&partial) {
            Ok(actual) if &actual == expected => {}
            Ok(_) => {
                return Err(fail(
                    "The download was corrupt (checksum mismatch) and was deleted.".into(),
                ));
            }
            Err(e) => return Err(fail(format!("Couldn't check the download: {e}"))),
        }
    }
    std::fs::rename(&partial, &target)
        .map_err(|e| fail(format!("Couldn't save the download: {e}")))?;
    Ok(target)
}

/// Shows a downloaded file to the user (opens the DMG on macOS; selects the
/// file in Explorer / the file manager elsewhere).
pub fn reveal(path: &Path) {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(path).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer").arg("/select,").arg(path).spawn()
    } else {
        Command::new("xdg-open")
            .arg(path.parent().unwrap_or(path))
            .spawn()
    };
    let _ = result;
}

/// Opens a web page in the default browser (or a folder in the file manager).
pub fn open_url(url: &str) {
    let _ = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        Command::new("explorer").arg(url).spawn()
    } else {
        Command::new("xdg-open").arg(url).spawn()
    };
}

/// The user's Downloads folder, falling back to the home folder.
pub fn downloads_dir() -> Option<PathBuf> {
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
    let downloads = home.join("Downloads");
    Some(if downloads.is_dir() { downloads } else { home })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha256::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish_hex())
}

/// SHA-256 (FIPS 180-4), small and streaming.
mod sha256 {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    pub struct Sha256 {
        state: [u32; 8],
        block: [u8; 64],
        filled: usize,
        length: u64,
    }

    impl Sha256 {
        pub fn new() -> Self {
            Sha256 {
                state: [
                    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                    0x1f83d9ab, 0x5be0cd19,
                ],
                block: [0; 64],
                filled: 0,
                length: 0,
            }
        }

        pub fn update(&mut self, mut data: &[u8]) {
            self.length += data.len() as u64;
            while !data.is_empty() {
                let take = (64 - self.filled).min(data.len());
                self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
                self.filled += take;
                data = &data[take..];
                if self.filled == 64 {
                    let block = self.block;
                    self.compress(&block);
                    self.filled = 0;
                }
            }
        }

        pub fn finish_hex(mut self) -> String {
            let bits = self.length.wrapping_mul(8);
            self.update(&[0x80]);
            while self.filled != 56 {
                self.update(&[0]);
            }
            self.update(&bits.to_be_bytes());
            self.state.iter().map(|w| format!("{w:08x}")).collect()
        }

        fn compress(&mut self, block: &[u8; 64]) {
            let mut w = [0u32; 64];
            for (w, chunk) in w.iter_mut().zip(block.as_chunks::<4>().0) {
                *w = u32::from_be_bytes(*chunk);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut v = self.state;
            for i in 0..64 {
                let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
                let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
                let t1 = v[7]
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
                let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
                let t2 = s0.wrapping_add(maj);
                v = [
                    t1.wrapping_add(t2),
                    v[0],
                    v[1],
                    v[2],
                    v[3].wrapping_add(t1),
                    v[4],
                    v[5],
                    v[6],
                ];
            }
            for (s, x) in self.state.iter_mut().zip(v) {
                *s = s.wrapping_add(x);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vectors() {
        let hex = |data: &[u8]| {
            let mut h = sha256::Sha256::new();
            h.update(data);
            h.finish_hex()
        };
        assert_eq!(
            hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // Streaming in odd-sized pieces gives the same digest.
        let data = vec![0x61u8; 1_000_000];
        let mut h = sha256::Sha256::new();
        for chunk in data.chunks(997) {
            h.update(chunk);
        }
        assert_eq!(
            h.finish_hex(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn version_comparison() {
        assert!(is_newer("0.2.0", "0.1.1"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.2.0-beta", "0.1.1"));
        assert!(!is_newer("garbage", "0.1.1"));
    }

    // Shape of GitHub's /releases/latest reply, shortened.
    const REPLY: &str = r#"{
      "tag_name": "v0.2.0", "html_url": "https://github.com/alidevecioglu/OpenCeres/releases/tag/v0.2.0",
      "body": "Notes with \"quotes\", a newline\n and é 🚀", "draft": false, "prerelease": false,
      "assets": [
        {"name": "OpenCeres-0.2.0-linux-x86_64.tar.gz", "size": 5008815, "digest": "sha256:AB12",
         "browser_download_url": "https://example.invalid/linux.tar.gz"},
        {"name": "OpenCeres-0.2.0-macos.dmg", "size": 7877583, "digest": "sha256:cd34",
         "browser_download_url": "https://example.invalid/mac.dmg"}
      ]
    }"#;

    #[test]
    fn parses_release_and_picks_platform_asset() {
        let r = parse_release(REPLY, Some("-macos.dmg")).unwrap();
        assert_eq!(r.version, "0.2.0");
        assert!(r.page.ends_with("/v0.2.0"));
        let asset = r.asset.unwrap();
        assert_eq!(
            (asset.name.as_str(), asset.size),
            ("OpenCeres-0.2.0-macos.dmg", 7_877_583)
        );
        assert_eq!(asset.sha256.as_deref(), Some("cd34"));
        let linux = parse_release(REPLY, Some("-linux-x86_64.tar.gz"))
            .unwrap()
            .asset
            .unwrap();
        assert_eq!(linux.sha256.as_deref(), Some("ab12"));
        assert_eq!(
            parse_release(REPLY, Some("-windows-x86_64.zip"))
                .unwrap()
                .asset,
            None
        );
        assert!(parse_release("not json", None).is_err());
    }

    #[test]
    fn json_strings_and_nesting() {
        let v = json::parse(REPLY).unwrap();
        assert_eq!(
            v.get("body").and_then(json::Value::as_str),
            Some("Notes with \"quotes\", a newline\n and é 🚀")
        );
        assert_eq!(v.get("draft"), Some(&json::Value::Bool(false)));
        assert!(json::parse("[1, 2").is_err());
        assert!(json::parse(&"[".repeat(100)).is_err()); // nesting limit
    }
}
