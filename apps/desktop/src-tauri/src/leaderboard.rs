//! The public leaderboard at leaderboard.xtrace.ai, which a person joins on
//! purpose from the Leaderboard page with a nickname and an email.
//!
//! * **What is sent.** Joining sends the nickname and the email. After that
//!   only the 7-day leverage number ([`leaderboard_tenths`]) is sent, right
//!   after joining, when the app starts and every hour while it runs
//!   ([`UPLOAD_INTERVAL`]); a failed upload is tried again at the next hour,
//!   with no pop-up. Renaming sends the new nickname; Leave asks the
//!   leaderboard to delete everything it has.
//! * **What is saved.** The key the leaderboard answers a join with, the
//!   nickname and the last number sent, in the app's own database
//!   (`xt_store::leaderboard`). The email is never saved. Neither the key nor
//!   the email is logged or put in an error message.
//! * **One rule each.** The server decides whether a nickname or email is
//!   valid, whether a member is listed and their rank; the app shows its
//!   answer in plain words ([`refusal_message`]). An `unknown_key` answer to
//!   any call means the leaderboard no longer has the membership: the saved
//!   one is forgotten and the page says so.
//! * **Separate from telemetry.** Nothing here reads the telemetry switch.
//!   A join only tells the telemetry service, which sends its own
//!   `leaderboard_opted_in` event only when telemetry may send.
//!
//! Fixture mode's service is not available: it never sends a fixture's
//! numbers to the public page.
use crate::dashboard_dto::MetricTile;
use crate::state::{AppState, StateError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use ts_rs::TS;
use xt_store::leaderboard::{LeaderboardMembership, SentScore};

/// The public leaderboard.
pub const DEFAULT_URL: &str = "https://leaderboard.xtrace.ai";
/// Replaces [`DEFAULT_URL`] for this launch (tests and local checks use a
/// local server). Only an `https://` address, or `http://` on this Mac, is
/// accepted; anything else leaves the default.
pub const URL_ENV: &str = "XTRACE_LEADERBOARD_URL";
/// The Dashboard range whose leverage is sent.
pub const LEADERBOARD_DAYS: u32 = 7;
/// How often the number is sent while the app runs, and how long a failed
/// upload waits before it is tried again.
pub const UPLOAD_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// The longest one request may take, connecting included.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// The longest connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest answer read; the leaderboard's are a few dozen bytes.
const MAX_ANSWER: usize = 64 * 1024;
/// What the app calls itself to the leaderboard.
const USER_AGENT: &str = concat!("XTrace-Desktop/", env!("CARGO_PKG_VERSION"));

/// The number the leaderboard gets: the Dashboard's 7-day leverage tile,
/// in tenths exactly as the screen writes it to one decimal
/// ([`crate::dashboard::written_tenths`]). None when the tile has no value
/// (no human time in the range), which the leaderboard shows as not listed.
pub fn leaderboard_tenths(leverage: &MetricTile) -> Option<u32> {
    leverage
        .value
        .and_then(crate::dashboard::written_tenths)
        .and_then(|tenths| u32::try_from(tenths).ok())
}

/// The leaderboard address this launch uses: `env` when it is an `https://`
/// origin, or an `http://` one on this Mac (`127.0.0.1`, `localhost`,
/// `[::1]`), with no path, query or credentials; [`DEFAULT_URL`] otherwise.
/// Never ends with `/`.
pub fn base_url(env: Option<&str>) -> String {
    env.and_then(|text| {
        let url = url::Url::parse(text.trim()).ok()?;
        let local = matches!(
            url.host(),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback()
        ) || matches!(url.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
            || url.host_str() == Some("localhost");
        let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && local);
        let plain = url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/"
            && url.host_str().is_some();
        (scheme_ok && plain).then(|| url.as_str().trim_end_matches('/').to_owned())
    })
    .unwrap_or_else(|| DEFAULT_URL.to_owned())
}

/// One request to the leaderboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub path: &'static str,
    /// Sent as `Authorization: Bearer <key>`.
    pub key: Option<String>,
    pub body: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// The leaderboard's answer: its status and body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

/// No answer arrived: not connected, refused, timed out, or too long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreachable;

/// Makes one request.
pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Answer, Unreachable>;
}

/// The app's HTTP client (the one "Add from a link" and the updater use):
/// no redirects, the time limits above, and no proxy for an address on this
/// Mac.
pub struct HttpTransport {
    client: reqwest::Client,
    base: String,
}

impl HttpTransport {
    pub fn new(base: &str) -> Option<Self> {
        // The same provider the updater installs; reqwest is built without one.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let local = base.starts_with("http://");
        let builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none());
        let builder = if local { builder.no_proxy() } else { builder };
        Some(Self {
            client: builder.build().ok()?,
            base: base.to_owned(),
        })
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: &Request) -> Result<Answer, Unreachable> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
        };
        let mut builder = self
            .client
            .request(method, format!("{}{}", self.base, request.path))
            .header("Accept", "application/json");
        if let Some(key) = &request.key {
            builder = builder.bearer_auth(key);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header("Content-Type", "application/json")
                .body(serde_json::to_vec(body).map_err(|_| Unreachable)?);
        }
        tauri::async_runtime::block_on(async move {
            let mut response = builder.send().await.map_err(|_| Unreachable)?;
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| Unreachable)? {
                if body.len() + chunk.len() > MAX_ANSWER {
                    return Err(Unreachable);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Answer { status, body })
        })
    }
}

/// Why a call did not do what was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No answer arrived.
    Unreachable,
    /// The leaderboard refused with one of its error codes.
    Code(String),
    /// Any other answer: an unexpected status or a body this build cannot read.
    Status(u16),
}

impl Refusal {
    /// The leaderboard no longer has this membership.
    fn gone(&self) -> bool {
        matches!(self, Self::Code(code) if code == "unknown_key")
    }
}

/// The plain words for each refusal, the one place they are written.
pub fn refusal_message(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Unreachable => {
            "Could not reach the leaderboard. Check your internet connection and try again.".into()
        }
        Refusal::Code(code) => match code.as_str() {
            "nickname_taken" => "That nickname is taken. Try another one.".into(),
            "invalid_nickname" => {
                "Nicknames are 3 to 20 characters: letters, digits, _ and - only.".into()
            }
            "invalid_email" => "That email address does not look right.".into(),
            "invalid_leverage" => "The leaderboard did not accept your number.".into(),
            "unknown_key" => GONE.into(),
            _ => "The leaderboard did not accept the request.".into(),
        },
        Refusal::Status(413) => "That nickname or email is too long.".into(),
        Refusal::Status(status) => {
            format!("The leaderboard answered with an error ({status}). Try again later.")
        }
    }
}

/// Shown once the leaderboard no longer knows the saved membership.
pub const GONE: &str =
    "The leaderboard no longer has your membership, so this app is not joined any more.";
/// Shown when the number could not be read from the app's own data.
const NUMBER_UNREADABLE: &str =
    "Your 7-day leverage could not be read. It will be tried again within the hour.";
/// Shown when an upload could not reach the leaderboard.
const UPLOAD_UNREACHABLE: &str =
    "Could not reach the leaderboard. Your number will be sent again within the hour.";

/// The leaderboard's answer to a join.
#[derive(Deserialize)]
struct Joined {
    key: String,
    nickname: String,
}

/// The leaderboard's answer about the member.
#[derive(Deserialize)]
struct Me {
    leverage_tenths: Option<u32>,
    listed: bool,
    rank: Option<u32>,
}

#[derive(Deserialize)]
struct Renamed {
    nickname: String,
}

/// The API calls of the contract, over a transport.
struct Api<'a>(&'a dyn Transport);

impl Api<'_> {
    fn call(&self, request: Request, ok: u16) -> Result<Vec<u8>, Refusal> {
        let answer = self.0.send(&request).map_err(|_| Refusal::Unreachable)?;
        if answer.status == ok {
            return Ok(answer.body);
        }
        #[derive(Deserialize)]
        struct Error {
            error: String,
        }
        match serde_json::from_slice::<Error>(&answer.body) {
            Ok(error) if (400..500).contains(&answer.status) && answer.status != 413 => {
                Err(Refusal::Code(error.error))
            }
            _ => Err(Refusal::Status(answer.status)),
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(body: &[u8], status: u16) -> Result<T, Refusal> {
        serde_json::from_slice(body).map_err(|_| Refusal::Status(status))
    }

    fn join(&self, nickname: &str, email: &str) -> Result<Joined, Refusal> {
        let body = self.call(
            Request {
                method: Method::Post,
                path: "/api/join",
                key: None,
                body: Some(json!({ "nickname": nickname, "email": email })),
            },
            201,
        )?;
        let joined: Joined = Self::parse(&body, 201)?;
        if joined.key.is_empty() || joined.key.len() > 256 || joined.nickname.is_empty() {
            return Err(Refusal::Status(201));
        }
        Ok(joined)
    }

    fn put_score(&self, key: &str, leverage_tenths: Option<u32>) -> Result<(), Refusal> {
        self.call(
            Request {
                method: Method::Put,
                path: "/api/score",
                key: Some(key.into()),
                body: Some(json!({ "leverage_tenths": leverage_tenths })),
            },
            204,
        )
        .map(drop)
    }

    fn me(&self, key: &str) -> Result<Me, Refusal> {
        let body = self.call(
            Request {
                method: Method::Get,
                path: "/api/me",
                key: Some(key.into()),
                body: None,
            },
            200,
        )?;
        Self::parse(&body, 200)
    }

    fn rename(&self, key: &str, nickname: &str) -> Result<Renamed, Refusal> {
        let body = self.call(
            Request {
                method: Method::Patch,
                path: "/api/me",
                key: Some(key.into()),
                body: Some(json!({ "nickname": nickname })),
            },
            200,
        )?;
        Self::parse(&body, 200)
    }

    fn delete(&self, key: &str) -> Result<(), Refusal> {
        self.call(
            Request {
                method: Method::Delete,
                path: "/api/me",
                key: Some(key.into()),
                body: None,
            },
            204,
        )
        .map(drop)
    }
}

/// What the service reads and writes in the app: the saved membership and
/// the 7-day leverage number.
pub trait Host: Send + Sync {
    fn membership(&self) -> Result<Option<LeaderboardMembership>, StateError>;
    fn join(&self, membership: &LeaderboardMembership) -> Result<(), StateError>;
    fn rename(&self, key: &str, nickname: &str) -> Result<(), StateError>;
    fn record(&self, key: &str, sent: SentScore) -> Result<(), StateError>;
    fn forget(&self, key: &str) -> Result<(), StateError>;
    /// [`leaderboard_tenths`] of the Dashboard's 7-day report, read now.
    fn leverage_tenths(&self) -> Result<Option<u32>, StateError>;
}

/// The running app's [`Host`]: its managed [`AppState`].
pub struct AppHost<R: tauri::Runtime>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> AppHost<R> {
    fn with<T>(
        &self,
        call: impl FnOnce(&AppState) -> Result<T, StateError>,
    ) -> Result<T, StateError> {
        use tauri::Manager;
        let state = self.0.try_state::<AppState>().ok_or(StateError::Closed)?;
        call(&state)
    }
}

impl<R: tauri::Runtime> Host for AppHost<R> {
    fn membership(&self) -> Result<Option<LeaderboardMembership>, StateError> {
        self.with(|state| state.with_store(|store| Ok(store.leaderboard_membership()?)))
    }
    fn join(&self, membership: &LeaderboardMembership) -> Result<(), StateError> {
        self.with(|state| {
            state.with_store(|store| Ok(store.join_leaderboard(membership).map(drop)?))
        })
    }
    fn rename(&self, key: &str, nickname: &str) -> Result<(), StateError> {
        self.with(|state| {
            state
                .with_store(|store| Ok(store.rename_leaderboard_member(key, nickname).map(drop)?))
        })
    }
    fn record(&self, key: &str, sent: SentScore) -> Result<(), StateError> {
        self.with(|state| {
            state.with_store(|store| Ok(store.record_leaderboard_score(key, sent).map(drop)?))
        })
    }
    fn forget(&self, key: &str) -> Result<(), StateError> {
        self.with(|state| state.with_store(|store| Ok(store.leave_leaderboard(key).map(drop)?)))
    }
    fn leverage_tenths(&self) -> Result<Option<u32>, StateError> {
        self.with(|state| {
            let report = state.metrics_dashboard(LEADERBOARD_DAYS)?;
            Ok(leaderboard_tenths(&report.tiles.leverage))
        })
    }
}

/// The last number the app sent and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct LeaderboardSent {
    /// The 7-day leverage in tenths (123 is written `12.3×`); none when the
    /// app had no 7-day number.
    pub leverage_tenths: Option<u32>,
    /// When the leaderboard accepted it, in Unix milliseconds.
    #[ts(type = "number")]
    pub at_ms: i64,
}

/// Where the member stands on the public page, as the leaderboard says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LeaderboardStanding {
    /// Listed, at this rank (equal numbers share a rank).
    Ranked { rank: u32 },
    /// On the leaderboard but not shown on the public page, and why.
    NotListed { reason: String },
    /// The leaderboard could not be asked.
    Unknown,
}

/// Everything the Leaderboard page shows. The page shows these fields and
/// never works any of them out again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct LeaderboardStatus {
    /// Whether this copy of the app can join (not in fixture mode).
    pub available: bool,
    pub joined: bool,
    pub nickname: Option<String>,
    pub last_sent: Option<LeaderboardSent>,
    /// Only while joined.
    pub standing: Option<LeaderboardStanding>,
    /// The public page, opened in the browser by `leaderboard_open_page`.
    pub page_url: String,
    /// Why the leaderboard could not be asked, or the last upload failed.
    pub error: Option<String>,
    /// A one-line note about a change the app made on its own, such as
    /// forgetting a membership the leaderboard no longer has.
    pub notice: Option<String>,
}

/// In-memory notes for the page; a restart starts with none.
#[derive(Default)]
struct Notes {
    upload_error: Option<String>,
    gone: bool,
}

/// Wall time in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// Called after a successful join.
pub type OnJoined = Arc<dyn Fn() + Send + Sync>;

/// The leaderboard service of one launch.
pub struct LeaderboardService {
    /// None when this launch cannot join (fixture mode, or no client).
    transport: Option<Arc<dyn Transport>>,
    host: Arc<dyn Host>,
    clock: Clock,
    on_joined: OnJoined,
    page_url: String,
    /// One join, rename, leave or upload at a time, so their saved results
    /// apply in the order they were asked for.
    work: Mutex<()>,
    notes: Mutex<Notes>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a saved-state failure says to the page.
fn saved_state_failed(_: StateError) -> String {
    "The leaderboard membership saved in this app could not be read or changed.".into()
}

impl LeaderboardService {
    pub fn new(
        base: &str,
        transport: Option<Arc<dyn Transport>>,
        host: Arc<dyn Host>,
        clock: Clock,
        on_joined: OnJoined,
    ) -> Self {
        Self {
            transport,
            host,
            clock,
            on_joined,
            page_url: format!("{base}/"),
            work: Mutex::new(()),
            notes: Mutex::new(Notes::default()),
        }
    }

    /// The public page; a compiled constant or [`URL_ENV`], never the
    /// renderer's.
    pub fn page_url(&self) -> &str {
        &self.page_url
    }

    fn api(&self) -> Result<Api<'_>, String> {
        self.transport
            .as_deref()
            .map(Api)
            .ok_or_else(|| "The leaderboard is not available in this copy of the app.".into())
    }

    /// Forgets the membership `key` names after the leaderboard said it no
    /// longer has it.
    fn forget_gone(&self, key: &str) {
        let _ = self.host.forget(key);
        let mut notes = lock(&self.notes);
        notes.gone = true;
        notes.upload_error = None;
    }

    /// Where things stand, asking the leaderboard for the rank when joined.
    pub fn status(&self) -> LeaderboardStatus {
        let membership = match self.host.membership() {
            Ok(membership) => membership,
            Err(error) => return self.answer(None, None, Some(saved_state_failed(error))),
        };
        let (Some(membership), Ok(api)) = (membership, self.api()) else {
            return self.answer(None, None, None);
        };
        match api.me(&membership.key) {
            Ok(me) => {
                let standing = match (me.listed, me.rank) {
                    (true, Some(rank)) => LeaderboardStanding::Ranked { rank },
                    _ if me.leverage_tenths.is_none() => LeaderboardStanding::NotListed {
                        reason: "Not listed yet: there is no leverage number for your last 7 days."
                            .into(),
                    },
                    _ => LeaderboardStanding::NotListed {
                        reason: "Not listed yet: your number has not reached the leaderboard in the last 48 hours."
                            .into(),
                    },
                };
                self.answer(Some(membership), Some(standing), None)
            }
            Err(refusal) if refusal.gone() => {
                self.forget_gone(&membership.key);
                self.answer(None, None, None)
            }
            Err(refusal) => self.answer(
                Some(membership),
                Some(LeaderboardStanding::Unknown),
                Some(refusal_message(&refusal)),
            ),
        }
    }

    fn answer(
        &self,
        membership: Option<LeaderboardMembership>,
        standing: Option<LeaderboardStanding>,
        error: Option<String>,
    ) -> LeaderboardStatus {
        let notes = lock(&self.notes);
        let joined = membership.is_some();
        LeaderboardStatus {
            available: self.transport.is_some(),
            joined,
            last_sent: membership
                .as_ref()
                .and_then(|m| m.last_sent)
                .map(|sent| LeaderboardSent {
                    leverage_tenths: sent.leverage_tenths,
                    at_ms: sent.at_ms,
                }),
            nickname: membership.map(|m| m.nickname),
            standing,
            page_url: self.page_url.clone(),
            error: error.or_else(|| joined.then(|| notes.upload_error.clone()).flatten()),
            notice: (notes.gone && !joined).then(|| GONE.into()),
        }
    }

    /// Joins with the nickname and email as typed (the leaderboard checks
    /// them), saves the key and nickname, then sends the number.
    pub fn join(&self, nickname: &str, email: &str) -> Result<LeaderboardStatus, String> {
        let api = self.api()?;
        {
            let _work = lock(&self.work);
            if self
                .host
                .membership()
                .map_err(saved_state_failed)?
                .is_some()
            {
                return Err("You have already joined the leaderboard.".into());
            }
            let joined = api
                .join(nickname, email)
                .map_err(|refusal| refusal_message(&refusal))?;
            let membership = LeaderboardMembership {
                key: joined.key,
                nickname: joined.nickname,
                last_sent: None,
            };
            if let Err(error) = self.host.join(&membership) {
                // Without the saved key the member could never leave, so the
                // new row is removed again.
                let _ = api.delete(&membership.key);
                return Err(saved_state_failed(error));
            }
            *lock(&self.notes) = Notes::default();
        }
        (self.on_joined)();
        self.upload();
        Ok(self.status())
    }

    /// Changes the nickname; the leaderboard checks it.
    pub fn rename(&self, nickname: &str) -> Result<LeaderboardStatus, String> {
        let api = self.api()?;
        {
            let _work = lock(&self.work);
            let membership = self.joined()?;
            match api.rename(&membership.key, nickname) {
                Ok(renamed) => self
                    .host
                    .rename(&membership.key, &renamed.nickname)
                    .map_err(saved_state_failed)?,
                Err(refusal) if refusal.gone() => self.forget_gone(&membership.key),
                Err(refusal) => return Err(refusal_message(&refusal)),
            }
        }
        Ok(self.status())
    }

    /// Asks the leaderboard to delete the member, then forgets the saved
    /// membership. When the leaderboard cannot be reached nothing is
    /// forgotten, so Leave can be tried again.
    pub fn leave(&self) -> Result<LeaderboardStatus, String> {
        let api = self.api()?;
        {
            let _work = lock(&self.work);
            let membership = self.joined()?;
            match api.delete(&membership.key) {
                Ok(()) => {}
                Err(refusal) if refusal.gone() => {}
                Err(refusal) => {
                    return Err(format!(
                        "{} You are still on the leaderboard.",
                        refusal_message(&refusal)
                    ));
                }
            }
            self.host
                .forget(&membership.key)
                .map_err(saved_state_failed)?;
            *lock(&self.notes) = Notes::default();
        }
        Ok(self.status())
    }

    fn joined(&self) -> Result<LeaderboardMembership, String> {
        self.host
            .membership()
            .map_err(saved_state_failed)?
            .ok_or_else(|| "You have not joined the leaderboard.".into())
    }

    /// Sends the current number when joined; does nothing otherwise. A
    /// failure is kept for the page and the next hour tries again.
    pub fn upload(&self) {
        let Ok(api) = self.api() else { return };
        let _work = lock(&self.work);
        let Ok(Some(membership)) = self.host.membership() else {
            return;
        };
        let tenths = match self.host.leverage_tenths() {
            Ok(tenths) => tenths,
            Err(_) => {
                lock(&self.notes).upload_error = Some(NUMBER_UNREADABLE.into());
                return;
            }
        };
        match api.put_score(&membership.key, tenths) {
            Ok(()) => {
                let sent = SentScore {
                    leverage_tenths: tenths,
                    at_ms: (self.clock)(),
                };
                let _ = self.host.record(&membership.key, sent);
                lock(&self.notes).upload_error = None;
            }
            Err(refusal) if refusal.gone() => self.forget_gone(&membership.key),
            Err(Refusal::Unreachable) => {
                lock(&self.notes).upload_error = Some(UPLOAD_UNREACHABLE.into());
            }
            Err(refusal) => lock(&self.notes).upload_error = Some(refusal_message(&refusal)),
        }
    }
}

/// Hands the public page to the system browser; the app window never
/// navigates.
pub fn open_page(url: &str) -> Result<(), StateError> {
    let status = page_command(url)?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| StateError::LeaderboardPageUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(StateError::LeaderboardPageUnavailable)
    }
}

/// An absolute launcher path, so the search path cannot substitute another.
#[cfg(target_os = "macos")]
fn page_command(url: &str) -> Result<std::process::Command, StateError> {
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg(url);
    Ok(command)
}
#[cfg(not(target_os = "macos"))]
fn page_command(_: &str) -> Result<std::process::Command, StateError> {
    Err(StateError::LeaderboardPageUnavailable)
}

#[cfg(test)]
#[path = "leaderboard_tests.rs"]
mod tests;
