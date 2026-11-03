//! OpenCeres Team: an optional team server (cloud or self-hosted) where a
//! teacher hands out image sets of a campaign and students submit their
//! detailed reports. With no server address the app never goes near it.
//!
//! Requests go through the system `curl`, like the SkyBoT lookup and the
//! update check, so no HTTP or TLS crate is linked. The request (address,
//! token, body) is written to curl's standard input as a config file
//! (`-K -`), never onto the command line, where other users of the
//! computer could read the token or password in the process list.
//!
//! Everything here blocks; the app calls it from background threads.

use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use crate::json::{self, Value};
use crate::prefs;

/// Whole-request limit; the connection itself must open within
/// [`CONNECT_SECS`].
const TIMEOUT_SECS: &str = "20";
const CONNECT_SECS: &str = "10";

// ---------------------------------------------------------------------------
// Settings

/// Server address and session, in `team.txt` in the data folder: never in
/// the history folder (which may be shared) or its exports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// e.g. `https://team.example.org`, without a trailing `/`; empty = offline.
    pub url: String,
    /// Session token from signing in; empty = signed out.
    pub token: String,
    /// Who is signed in, to show without asking the server.
    pub name: String,
    pub email: String,
}

impl Config {
    pub fn signed_in(&self) -> bool {
        !self.url.is_empty() && !self.token.is_empty()
    }

    pub fn sign_out(&mut self) {
        self.token.clear();
        self.name.clear();
        self.email.clear();
    }

    pub fn load() -> Config {
        path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Config::default, |t| Config::parse(&t))
    }

    fn parse(text: &str) -> Config {
        let mut c = Config::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_owned();
            match key.trim() {
                "url" => c.url = normalize_url(&value).unwrap_or_default(),
                "token" => c.token = value,
                "name" => c.name = value,
                "email" => c.email = value,
                _ => {}
            }
        }
        if c.url.is_empty() {
            c.sign_out();
        }
        c
    }

    fn to_text(&self) -> String {
        let clean = |s: &str| s.replace(['\n', '\r'], " ");
        format!(
            "url={}\ntoken={}\nname={}\nemail={}\n",
            clean(&self.url),
            clean(&self.token),
            clean(&self.name),
            clean(&self.email)
        )
    }

    /// Best effort, like the preferences. Readable by this user only, as
    /// it holds the session token.
    pub fn save(&self) {
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(path) {
            let _ = file.write_all(self.to_text().as_bytes());
        }
    }
}

fn path() -> Option<PathBuf> {
    prefs::data_dir().map(|d| d.join("team.txt"))
}

/// A typed server address made usable: `https://` added when no scheme is
/// given, trailing `/` dropped. Empty stays empty (offline).
pub fn normalize_url(input: &str) -> Result<String, String> {
    let url = input.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let url = if url.contains("://") {
        url.to_owned()
    } else {
        format!("https://{url}")
    };
    let lower = url.to_ascii_lowercase();
    let scheme = if lower.starts_with("https://") {
        "https://".len()
    } else if lower.starts_with("http://") {
        "http://".len()
    } else {
        return Err("The address must start with https:// or http://.".into());
    };
    let url = format!("{}{}", &url[..scheme], url[scheme..].trim_end_matches('/'));
    if url.len() == scheme
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\\')
    {
        return Err("That doesn't look like a server address.".into());
    }
    Ok(url)
}

/// Plain `http://` to anything but this computer sends the password and
/// token unencrypted.
pub fn is_insecure(url: &str) -> bool {
    let Some(rest) = url
        .to_ascii_lowercase()
        .strip_prefix("http://")
        .map(str::to_owned)
    else {
        return false;
    };
    if rest.starts_with("[::1]") {
        return false;
    }
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    !matches!(host, "localhost" | "127.0.0.1")
}

// ---------------------------------------------------------------------------
// What the server sends

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub api: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
    pub must_change_password: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TeamRef {
    pub id: String,
    pub name: String,
    /// `teacher` or `student`; empty where the server does not say.
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Me {
    pub user: User,
    pub teams: Vec<TeamRef>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Campaign {
    pub id: String,
    pub name: String,
    pub starts_on: String,
    pub ends_on: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Assignment {
    pub set_id: String,
    pub code: String,
    pub status: Option<String>,
    pub campaign: Campaign,
    pub team: TeamRef,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Announcement {
    pub id: String,
    pub team_name: String,
    pub title: String,
    pub body: String,
    pub author: String,
    /// Unix time.
    pub created_at: u64,
}

/// A campaign set the opened files belong to.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Match {
    pub set_id: String,
    pub code: String,
    pub status: Option<String>,
    /// Assigned to the signed-in user to review.
    pub assigned: bool,
    pub role: String,
    pub campaign: Campaign,
    pub team: TeamRef,
}

impl Match {
    /// e.g. `June 2026 · XY57`.
    pub fn badge(&self) -> String {
        match (self.campaign.name.trim(), self.code.trim()) {
            ("", code) => code.to_owned(),
            (campaign, "") => campaign.to_owned(),
            (campaign, code) => format!("{campaign} · {code}"),
        }
    }
}

/// The set to show when the files match several (the same images in two
/// campaigns or teams): one assigned to this user, else one they teach,
/// else the first.
pub fn pick_match(matches: &[Match]) -> Option<&Match> {
    let rank = |m: &Match| u8::from(m.assigned) * 2 + u8::from(m.role == "teacher");
    let best = matches.iter().map(rank).max()?;
    matches.iter().find(|m| rank(m) == best)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub submitted_at: Option<u64>,
    /// The detailed report as the app exported it.
    pub report: Value,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Submissions {
    pub list: Vec<Submission>,
    /// The campaign shows others' reviews only after one's own.
    pub hidden_until_you_submit: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Submitted {
    pub id: String,
    pub submitted_at: Option<u64>,
}

/// Why a request failed, as far as the app needs to act on it.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The session is no longer valid (401): sign in again.
    SignedOut(String),
    /// An admin reset the password: a new one must be chosen first (403).
    MustChangePassword,
    Other(String),
}

impl ApiError {
    pub fn message(&self) -> String {
        match self {
            ApiError::SignedOut(m) | ApiError::Other(m) => m.clone(),
            ApiError::MustChangePassword => "Choose a new password to continue.".into(),
        }
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Ids may be numbers or strings; both become text.
fn id(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{}", *n as i64),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn flag(v: &Value, key: &str) -> bool {
    matches!(v.get(key), Some(Value::Bool(true)))
}

fn unix(v: &Value, key: &str) -> Option<u64> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|n| *n >= 0.0)
        .map(|n| n as u64)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).unwrap_or(&[])
}

fn parse_user(v: &Value) -> User {
    User {
        id: id(v, "id"),
        name: text(v, "name"),
        email: text(v, "email"),
        is_admin: flag(v, "is_admin"),
        must_change_password: flag(v, "must_change_password"),
    }
}

fn parse_team(v: &Value) -> TeamRef {
    TeamRef {
        id: id(v, "id"),
        name: text(v, "name"),
        role: text(v, "role"),
    }
}

fn parse_campaign(v: &Value) -> Campaign {
    Campaign {
        id: id(v, "id"),
        name: text(v, "name"),
        starts_on: text(v, "starts_on"),
        ends_on: text(v, "ends_on"),
    }
}

fn sub(v: &Value, key: &str) -> Value {
    v.get(key).cloned().unwrap_or(Value::Null)
}

pub fn parse_info(v: &Value) -> ApiResult<ServerInfo> {
    if text(v, "name") != "openceres-team" {
        return Err(ApiError::Other(
            "That server doesn't look like an OpenCeres Team server.".into(),
        ));
    }
    Ok(ServerInfo {
        name: text(v, "name"),
        version: text(v, "version"),
        api: v.get("api").and_then(Value::as_f64).unwrap_or(0.0) as u32,
    })
}

pub fn parse_login(v: &Value) -> ApiResult<(String, User)> {
    let token = text(v, "token");
    if token.is_empty() {
        return Err(unreadable());
    }
    Ok((token, parse_user(&sub(v, "user"))))
}

pub fn parse_me(v: &Value) -> Me {
    Me {
        user: parse_user(&sub(v, "user")),
        teams: list(v, "teams").iter().map(parse_team).collect(),
    }
}

pub fn parse_assignments(v: &Value) -> Vec<Assignment> {
    list(v, "assignments")
        .iter()
        .map(|a| Assignment {
            set_id: id(a, "set_id"),
            code: text(a, "code"),
            status: a.get("status").and_then(Value::as_str).map(str::to_owned),
            campaign: parse_campaign(&sub(a, "campaign")),
            team: parse_team(&sub(a, "team")),
        })
        .filter(|a| !a.set_id.is_empty())
        .collect()
}

pub fn parse_announcements(v: &Value) -> Vec<Announcement> {
    list(v, "announcements")
        .iter()
        .map(|a| Announcement {
            id: id(a, "id"),
            team_name: text(a, "team_name"),
            title: text(a, "title"),
            body: text(a, "body"),
            author: text(a, "author"),
            created_at: unix(a, "created_at").unwrap_or(0),
        })
        .collect()
}

pub fn parse_matches(v: &Value) -> Vec<Match> {
    list(v, "matches")
        .iter()
        .map(|m| Match {
            set_id: id(m, "set_id"),
            code: text(m, "code"),
            status: m.get("status").and_then(Value::as_str).map(str::to_owned),
            assigned: flag(m, "assigned"),
            role: text(m, "role"),
            campaign: parse_campaign(&sub(m, "campaign")),
            team: parse_team(&sub(m, "team")),
        })
        .filter(|m| !m.set_id.is_empty())
        .collect()
}

pub fn parse_submissions(v: &Value) -> Submissions {
    Submissions {
        list: list(v, "submissions")
            .iter()
            .filter_map(|s| {
                let report = s.get("report")?;
                matches!(report, Value::Object(_)).then(|| {
                    let user = sub(s, "user");
                    Submission {
                        id: id(s, "id"),
                        user_id: id(&user, "id"),
                        user_name: text(&user, "name"),
                        submitted_at: unix(s, "submitted_at"),
                        report: report.clone(),
                    }
                })
            })
            .collect(),
        hidden_until_you_submit: flag(v, "hidden_until_you_submit"),
    }
}

pub fn parse_submitted(v: &Value) -> Submitted {
    Submitted {
        id: id(v, "id"),
        submitted_at: unix(v, "submitted_at"),
    }
}

fn unreadable() -> ApiError {
    ApiError::Other("The server's answer couldn't be read.".into())
}

// ---------------------------------------------------------------------------
// HTTP

/// A request as handed to curl.
#[derive(Debug, Clone, PartialEq)]
pub struct Request<'a> {
    pub method: &'a str,
    pub url: String,
    pub token: Option<&'a str>,
    /// JSON.
    pub body: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    /// Where a 3xx answer points.
    pub redirect: String,
}

/// A string for curl's config syntax: double-quoted, with `\` escapes.
fn config_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The config file curl reads from standard input for `request`.
pub fn curl_config(request: &Request) -> String {
    let mut lines = vec![
        format!("url = {}", config_quote(&request.url)),
        format!("request = {}", config_quote(request.method)),
        format!(
            "header = {}",
            config_quote(&format!(
                "User-Agent: OpenCeres/{}",
                crate::update::CURRENT_VERSION
            ))
        ),
        "header = \"Accept: application/json\"".to_owned(),
    ];
    if let Some(token) = request.token {
        lines.push(format!(
            "header = {}",
            config_quote(&format!("Authorization: Bearer {token}"))
        ));
    }
    if let Some(body) = request.body {
        lines.push("header = \"Content-Type: application/json\"".into());
        // No "Expect: 100-continue" round trip for larger reports.
        lines.push("header = \"Expect:\"".into());
        lines.push(format!("data-raw = {}", config_quote(body)));
    }
    lines.join("\n") + "\n"
}

/// Sends `request` and returns the HTTP answer, whatever its status.
pub fn send(request: &Request) -> Result<Reply, String> {
    let mut child = crate::known::curl()
        .args(["-sS", "-K", "-", "--proto", "=http,https"])
        .args([
            "--max-time",
            TIMEOUT_SECS,
            "--connect-timeout",
            CONNECT_SECS,
        ])
        .args(["-w", "\n%{http_code} %{redirect_url}"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "Couldn't run curl, which OpenCeres uses to go online.".to_owned())?;
    if let Some(mut stdin) = child.stdin.take() {
        // curl reads the whole config before it connects.
        let _ = stdin.write_all(curl_config(request).as_bytes());
    }
    let output = child
        .wait_with_output()
        .map_err(|_| "curl stopped unexpectedly.".to_owned())?;
    if !output.status.success() {
        return Err(curl_error(output.status.code(), &request.url));
    }
    parse_reply(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "The server's answer couldn't be read.".to_owned())
}

/// Splits curl's output into the body and the `-w` trailer.
fn parse_reply(out: &str) -> Option<Reply> {
    let (body, trailer) = out.rsplit_once('\n')?;
    let (status, redirect) = trailer.split_once(' ').unwrap_or((trailer, ""));
    Some(Reply {
        status: status.trim().parse().ok()?,
        body: body.to_owned(),
        redirect: redirect.trim().to_owned(),
    })
}

fn curl_error(code: Option<i32>, url: &str) -> String {
    match code {
        Some(1 | 3) => "That server address isn't valid.".into(),
        Some(5 | 6) => format!(
            "Couldn't find the server {} — check the address and the internet connection.",
            host(url)
        ),
        Some(7) => "Couldn't connect to the server — check that it is running and the \
                    address is right."
            .into(),
        Some(28) => "The server took too long to answer. Try again later.".into(),
        Some(35 | 51 | 53 | 54 | 58 | 59 | 60 | 64 | 66 | 77 | 80 | 82 | 83 | 90 | 91) => {
            "Couldn't make a secure connection to the server (certificate problem).".into()
        }
        Some(52 | 56) => "The server closed the connection without an answer.".into(),
        Some(n) => format!("Couldn't reach the server (curl error {n})."),
        None => "curl stopped unexpectedly.".into(),
    }
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest)
}

/// The answer as JSON, or why the request failed.
fn check(reply: &Reply) -> ApiResult<Value> {
    let parsed = if reply.body.trim().is_empty() {
        Ok(Value::Null)
    } else {
        json::parse(&reply.body)
    };
    let message = parsed
        .as_ref()
        .ok()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    match reply.status {
        200..=299 => parsed.map_err(|()| unreadable()),
        300..=399 => Err(ApiError::Other(if reply.redirect.is_empty() {
            "The server sent the request elsewhere; check its address.".into()
        } else {
            format!(
                "The server moved to {}; use that address.",
                reply.redirect.split("/api/v1").next().unwrap_or_default()
            )
        })),
        401 => Err(ApiError::SignedOut(message.unwrap_or_else(|| {
            "You were signed out. Please sign in again.".into()
        }))),
        403 if parsed
            .as_ref()
            .is_ok_and(|v| flag(v, "must_change_password")) =>
        {
            Err(ApiError::MustChangePassword)
        }
        404 if message.is_none() => Err(ApiError::Other(
            "The server doesn't know this request — is it an OpenCeres Team server, and \
             up to date?"
                .into(),
        )),
        413 => Err(ApiError::Other(
            "The report is too large for the server (over 2 MB).".into(),
        )),
        code => Err(ApiError::Other(
            message.unwrap_or_else(|| format!("The server answered {code}.")),
        )),
    }
}

/// Percent-encodes an id for use in a path.
fn segment(id: &str) -> String {
    id.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// One API call: `base` is the server address, `path` starts with `/`.
fn call(
    base: &str,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> ApiResult<Value> {
    let request = Request {
        method,
        url: format!("{base}/api/v1{path}"),
        token,
        body,
    };
    check(&send(&request).map_err(ApiError::Other)?)
}

// ---------------------------------------------------------------------------
// The API

pub fn info(base: &str) -> ApiResult<ServerInfo> {
    parse_info(&call(base, None, "GET", "/info", None)?)
}

pub fn login(base: &str, email: &str, password: &str) -> ApiResult<(String, User)> {
    let body = format!(
        "{{\"email\": {}, \"password\": {}}}",
        json::quote(email.trim()),
        json::quote(password)
    );
    match call(base, None, "POST", "/auth/login", Some(&body)) {
        Ok(v) => parse_login(&v),
        Err(ApiError::SignedOut(_)) => Err(ApiError::Other("Wrong email or password.".into())),
        Err(e) => Err(e),
    }
}

pub fn logout(base: &str, token: &str) -> ApiResult<()> {
    call(base, Some(token), "POST", "/auth/logout", None).map(|_| ())
}

pub fn me(base: &str, token: &str) -> ApiResult<Me> {
    call(base, Some(token), "GET", "/me", None).map(|v| parse_me(&v))
}

pub fn change_password(base: &str, token: &str, current: &str, new: &str) -> ApiResult<()> {
    let body = format!(
        "{{\"current\": {}, \"new\": {}}}",
        json::quote(current),
        json::quote(new)
    );
    call(base, Some(token), "POST", "/account/password", Some(&body)).map(|_| ())
}

pub fn assignments(base: &str, token: &str) -> ApiResult<Vec<Assignment>> {
    call(base, Some(token), "GET", "/assignments", None).map(|v| parse_assignments(&v))
}

pub fn announcements(base: &str, token: &str) -> ApiResult<Vec<Announcement>> {
    call(base, Some(token), "GET", "/announcements", None).map(|v| parse_announcements(&v))
}

/// The server takes at most this many names per lookup.
const LOOKUP_MAX: usize = 500;

pub fn lookup(base: &str, token: &str, files: &[String]) -> ApiResult<Vec<Match>> {
    let files = &files[..files.len().min(LOOKUP_MAX)];
    let body = format!("{{\"files\": {}}}", json::string_array(files));
    call(base, Some(token), "POST", "/sets/lookup", Some(&body)).map(|v| parse_matches(&v))
}

pub fn submissions(base: &str, token: &str, set_id: &str) -> ApiResult<Submissions> {
    let path = format!("/sets/{}/submissions", segment(set_id));
    call(base, Some(token), "GET", &path, None).map(|v| parse_submissions(&v))
}

/// Sends a detailed report (the JSON the app exports, without cutouts).
pub fn submit(base: &str, token: &str, set_id: &str, report: &str) -> ApiResult<Submitted> {
    let path = format!("/sets/{}/submissions", segment(set_id));
    call(base, Some(token), "POST", &path, Some(report)).map(|v| parse_submitted(&v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    fn parse(text: &str) -> Value {
        json::parse(text).unwrap()
    }

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.into(),
            redirect: String::new(),
        }
    }

    #[test]
    fn config_round_trips_and_needs_an_address() {
        let c = Config {
            url: "https://team.example.org".into(),
            token: "abc.def".into(),
            name: "Ayşe".into(),
            email: "ayse@example.org".into(),
        };
        assert_eq!(Config::parse(&c.to_text()), c);
        assert!(c.signed_in());
        let c = Config::parse("url=\ntoken=abc\nname=x\n");
        assert_eq!(c, Config::default());
        assert!(!c.signed_in());
    }

    #[test]
    fn addresses() {
        assert_eq!(normalize_url("  ").unwrap(), "");
        assert_eq!(
            normalize_url("team.example.org/").unwrap(),
            "https://team.example.org"
        );
        assert_eq!(
            normalize_url("http://localhost:8080").unwrap(),
            "http://localhost:8080"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("https://a b").is_err());
        assert!(normalize_url("https://").is_err());
        assert!(is_insecure("http://team.example.org"));
        assert!(!is_insecure("http://localhost:8080/x"));
        assert!(!is_insecure("https://team.example.org"));
    }

    #[test]
    fn parses_info_login_and_me() {
        let info = parse_info(&parse(
            r#"{"name":"openceres-team","version":"0.1.0","api":1,"registration":"invite"}"#,
        ))
        .unwrap();
        assert_eq!((info.version.as_str(), info.api), ("0.1.0", 1));
        assert!(parse_info(&parse(r#"{"name":"nginx"}"#)).is_err());

        let (token, user) = parse_login(&parse(
            r#"{"token":"t0k","user":{"id":7,"name":"Ali","email":"a@x.org",
                "is_admin":false,"must_change_password":true}}"#,
        ))
        .unwrap();
        assert_eq!(token, "t0k");
        assert_eq!((user.id.as_str(), user.name.as_str()), ("7", "Ali"));
        assert!(user.must_change_password && !user.is_admin);
        assert!(parse_login(&parse(r#"{"user":{}}"#)).is_err());

        let me = parse_me(&parse(
            r#"{"user":{"id":"u1","name":"Ali"},"teams":[{"id":3,"name":"Astro club","role":"teacher"}]}"#,
        ));
        assert_eq!(me.user.id, "u1");
        assert_eq!(me.teams[0].name, "Astro club");
        assert_eq!(me.teams[0].role, "teacher");
    }

    #[test]
    fn parses_lists_and_skips_junk() {
        let a = parse_assignments(&parse(
            r#"{"assignments":[{"set_id":12,"code":"XY57","status":null,
                "campaign":{"id":1,"name":"June 2026","starts_on":"2026-06-01","ends_on":"2026-06-30"},
                "team":{"id":3,"name":"Astro club"}}, {"code":"no id"}, 5]}"#,
        ));
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].set_id.as_str(), a[0].code.as_str()), ("12", "XY57"));
        assert_eq!(a[0].status, None);
        assert_eq!(a[0].campaign.ends_on, "2026-06-30");

        let n = parse_announcements(&parse(
            r#"{"announcements":[{"id":1,"team_id":3,"team_name":"Astro club","title":"Hi",
                "body":"Line 1\nLine 2","author":"Ms K","created_at":1793000000}]}"#,
        ));
        assert_eq!(n[0].created_at, 1_793_000_000);
        assert_eq!(n[0].body, "Line 1\nLine 2");
        assert!(parse_announcements(&parse("{}")).is_empty());
        assert!(parse_assignments(&parse("[]")).is_empty());
    }

    #[test]
    fn picks_the_assigned_match_for_the_badge() {
        let m = parse_matches(&parse(
            r#"{"matches":[
                {"set_id":1,"code":"XY57","status":null,"assigned":false,"role":"student",
                 "campaign":{"name":"May 2026"},"team":{"name":"A"}},
                {"set_id":2,"code":"XY57","status":"open","assigned":false,"role":"teacher",
                 "campaign":{"name":"June 2026"},"team":{"name":"B"}},
                {"set_id":3,"code":"XY57","status":"open","assigned":true,"role":"student",
                 "campaign":{"name":"July 2026"},"team":{"name":"C"}}]}"#,
        ));
        assert_eq!(m.len(), 3);
        let best = pick_match(&m).unwrap();
        assert_eq!(best.set_id, "3");
        assert_eq!(best.badge(), "July 2026 · XY57");
        assert_eq!(pick_match(&m[..2]).unwrap().set_id, "2");
        assert_eq!(pick_match(&m[..1]).unwrap().set_id, "1");
        assert_eq!(pick_match(&[]), None);
        let no_campaign = Match {
            code: "XY58".into(),
            ..Match::default()
        };
        assert_eq!(no_campaign.badge(), "XY58");
    }

    #[test]
    fn parses_submissions_with_reports() {
        let s = parse_submissions(&parse(
            r#"{"set":{"id":3,"code":"XY57"},"submissions":[
                {"id":9,"user":{"id":7,"name":"Ali"},"submitted_at":1793000000,
                 "report":{"format":"openceres-search","id":"XY57-1-abc","set":"XY57"}},
                {"id":10,"user":{"id":8,"name":"Bad"},"report":"not an object"}],
                "hidden_until_you_submit":true}"#,
        ));
        assert!(s.hidden_until_you_submit);
        assert_eq!(s.list.len(), 1);
        assert_eq!(s.list[0].user_name, "Ali");
        assert_eq!(s.list[0].submitted_at, Some(1_793_000_000));
        let record = crate::history::Record::from_value(&s.list[0].report).unwrap();
        assert_eq!(record.set, "XY57");
        assert!(!parse_submissions(&parse(r#"{"submissions":[]}"#)).hidden_until_you_submit);
    }

    #[test]
    fn errors_from_status_codes() {
        assert_eq!(check(&reply(204, "")), Ok(Value::Null));
        assert!(check(&reply(200, "<html>")).is_err());
        assert_eq!(
            check(&reply(401, r#"{"error":"token expired"}"#)),
            Err(ApiError::SignedOut("token expired".into()))
        );
        assert_eq!(
            check(&reply(
                403,
                r#"{"error":"password change required","must_change_password":true}"#
            )),
            Err(ApiError::MustChangePassword)
        );
        assert_eq!(
            check(&reply(403, r#"{"error":"not in this team"}"#)),
            Err(ApiError::Other("not in this team".into()))
        );
        assert_eq!(
            check(&reply(400, r#"{"error":"password too short"}"#)),
            Err(ApiError::Other("password too short".into()))
        );
        assert!(matches!(check(&reply(413, "")), Err(ApiError::Other(m)) if m.contains("2 MB")));
        assert!(matches!(check(&reply(502, "<html>bad gateway</html>")),
            Err(ApiError::Other(m)) if m.contains("502")));
        let moved = Reply {
            status: 301,
            body: String::new(),
            redirect: "https://team.example.org/api/v1/info".into(),
        };
        assert!(
            matches!(check(&moved), Err(ApiError::Other(m)) if m.contains("to https://team.example.org;"))
        );
    }

    #[test]
    fn curl_output_and_config() {
        let r = parse_reply("{\"a\":1}\n\n201 ").unwrap();
        assert_eq!((r.status, r.body.as_str()), (201, "{\"a\":1}\n"));
        assert!(parse_reply("no trailer").is_none());

        let config = curl_config(&Request {
            method: "POST",
            url: "https://t.example/api/v1/x".into(),
            token: Some("tok\"en"),
            body: Some("{\n  \"notes\": \"a \\\"b\\\" c\"\n}"),
        });
        assert!(config.contains("url = \"https://t.example/api/v1/x\"\n"));
        assert!(config.contains("header = \"Authorization: Bearer tok\\\"en\"\n"));
        assert!(config.contains(r#"data-raw = "{\n  \"notes\": \"a \\\"b\\\" c\"\n}""#));
        assert_eq!(config.lines().count(), 8, "{config}");
        assert_eq!(segment("12"), "12");
        assert_eq!(segment("a/b c"), "a%2Fb%20c");
    }

    /// Reads one HTTP request (headers and a `Content-Length` body).
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut reader = BufReader::new(stream);
        let mut head = String::new();
        let mut length = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            }
            head += &line;
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        head + "\r\n" + &String::from_utf8_lossy(&body)
    }

    /// The whole path through curl, against a one-shot local server.
    #[test]
    fn submits_through_curl() {
        if crate::known::curl().arg("--version").output().is_err() {
            eprintln!("curl is not installed; skipping");
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = r#"{"id": 41, "submitted_at": 1793000000}"#;
            let answer = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(answer.as_bytes()).unwrap();
            request
        });
        let report = "{\n  \"set\": \"XY57\",\n  \"notes\": \"Ayşe \\\"x\\\" \\\\ y\"\n}";
        let done = submit(&base, "secret-token", "12", report).unwrap();
        assert_eq!(
            done,
            Submitted {
                id: "41".into(),
                submitted_at: Some(1_793_000_000)
            }
        );
        let request = server.join().unwrap();
        assert!(
            request.starts_with("POST /api/v1/sets/12/submissions HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains("Authorization: Bearer secret-token\r\n"));
        assert!(request.contains("Content-Type: application/json\r\n"));
        assert!(!request.contains("Expect:"));
        assert!(request.ends_with(&format!("\r\n\r\n{report}")), "{request}");
    }

    #[test]
    fn signed_out_and_unreachable_through_curl() {
        if crate::known::curl().arg("--version").output().is_err() {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = r#"{"error":"invalid token"}"#;
            let answer = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(answer.as_bytes()).unwrap();
            request
        });
        assert_eq!(
            me(&base, "old"),
            Err(ApiError::SignedOut("invalid token".into()))
        );
        assert!(
            server
                .join()
                .unwrap()
                .starts_with("GET /api/v1/me HTTP/1.1\r\n")
        );
        // Nothing listens there any more.
        assert!(matches!(info(&base), Err(ApiError::Other(m)) if m.contains("connect")));
    }
}
