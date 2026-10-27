//! The public leaderboard at leaderboard.xtrace.ai, which a person joins on
//! purpose from the Leaderboard page with a nickname and an email.
//!
//! * **What is sent.** Joining sends the nickname and the email; the
//!   leaderboard emails a 6-digit code, and the code is sent back to finish
//!   (the pending join's id is kept in memory only). After that
//!   only the 7-day leverage number ([`leaderboard_tenths`]) is sent, right
//!   after joining, when the app starts, when its index is first ready and
//!   every hour while it runs ([`UPLOAD_INTERVAL`]); a failed upload is tried again at the next hour,
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
//! * **Which copies send.** Only the installed app on its own live data
//!   ([`crate::telemetry::installed_own_data`], the rule telemetry uses), or
//!   any copy whose [`URL_ENV`] names a server on this Mac. A test copy of a
//!   database keeps the saved key, so any other copy sends nothing
//!   ([`may_send`]).
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
        let scheme_ok = url.scheme() == "https" || (url.scheme() == "http" && on_this_mac(&url));
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

/// Whether an address names a server on this Mac (`127.0.0.1` and the rest
/// of the loopback range, `[::1]` or `localhost`).
fn on_this_mac(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name == "localhost",
        None => false,
    }
}

/// Whether this launch may send anything to the leaderboard at `base` (from
/// [`base_url`]): the installed app on its own live data
/// (`installed_own_data`, from [`crate::telemetry::installed_own_data`]), or
/// any copy pointed at a server on this Mac.
pub fn may_send(installed_own_data: bool, base: &str) -> bool {
    installed_own_data || url::Url::parse(base).is_ok_and(|url| on_this_mac(&url))
}

/// Shown, and nothing is sent, in a copy [`may_send`] refuses.
pub const BLOCKED_COPY: &str = "This copy of XTrace does not send to the leaderboard.";
/// Shown when the app's network client could not start.
pub const NO_CLIENT: &str =
    "XTrace could not start its connection to the leaderboard. Restart the app to try again.";

/// One request to the leaderboard. Its `Debug` hides the key and every
/// value of the body (the email among them).
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub path: &'static str,
    /// Sent as `Authorization: Bearer <key>`.
    pub key: Option<String>,
    pub body: Option<Value>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fields: Vec<&str> = self
            .body
            .as_ref()
            .and_then(Value::as_object)
            .map(|body| body.keys().map(String::as_str).collect())
            .unwrap_or_default();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("key", &self.key.as_ref().map(|_| "<hidden>"))
            .field("body_fields", &fields)
            .finish()
    }
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
    /// Too many requests; try again after this many seconds, when it said.
    RateLimited(Option<u64>),
    /// Any other answer status.
    Status(u16),
    /// The expected status with a body this build cannot read.
    Unreadable,
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
            "invalid_code" => "That code is not right. Check the email and try again.".into(),
            "code_expired" => {
                "That code has expired or was tried too many times. Send a new code.".into()
            }
            "email_unavailable" => {
                "The leaderboard can't send emails right now. Try again later.".into()
            }
            "unknown_key" => GONE.into(),
            _ => "The leaderboard did not accept the request.".into(),
        },
        Refusal::RateLimited(Some(seconds)) => {
            // Whole minutes, rounded up, at least one.
            let minutes = seconds.div_ceil(60).max(1);
            let unit = if minutes == 1 { "minute" } else { "minutes" };
            format!("Too many tries. Try again in about {minutes} {unit}.")
        }
        Refusal::RateLimited(None) => "Too many tries. Try again later.".into(),
        Refusal::Status(413) => "That nickname or email is too long.".into(),
        Refusal::Unreadable => "The leaderboard answered in a way XTrace could not read.".into(),
        Refusal::Status(status) => {
            format!("The leaderboard answered with an error ({status}). Try again later.")
        }
    }
}

/// The form field a refusal is about, so the page can mark it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardField {
    Nickname,
    Email,
    Code,
}

/// A refused change, as the page shows it: the words, and the field they
/// are about when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct LeaderboardRefusal {
    pub message: String,
    pub field: Option<LeaderboardField>,
}

impl From<String> for LeaderboardRefusal {
    fn from(message: String) -> Self {
        Self {
            message,
            field: None,
        }
    }
}

impl From<&Refusal> for LeaderboardRefusal {
    fn from(refusal: &Refusal) -> Self {
        let field = match refusal {
            Refusal::Code(code) => match code.as_str() {
                "nickname_taken" | "invalid_nickname" => Some(LeaderboardField::Nickname),
                "invalid_email" => Some(LeaderboardField::Email),
                "invalid_code" => Some(LeaderboardField::Code),
                _ => None,
            },
            _ => None,
        };
        Self {
            message: refusal_message(refusal),
            field,
        }
    }
}

/// Shown once the leaderboard no longer knows the saved membership.
pub const GONE: &str =
    "The leaderboard no longer has your membership, so this app is not joined any more.";
/// Shown when the number could not be read from the app's own data.
const NUMBER_UNREADABLE: &str =
    "Your 7-day leverage could not be read. It will be tried again within the hour.";
/// Shown when the number was accepted but its record could not be saved.
const RECORD_FAILED: &str = "Your number was sent, but XTrace could not save when it was sent.";
/// Shown when the leaderboard asked the hourly upload to wait.
const UPLOAD_RATE_LIMITED: &str =
    "The leaderboard asked XTrace to wait. Your number will be sent again within the hour.";
/// Shown when a join was accepted but its key could not be saved, and the
/// entry could not be removed again either.
const JOIN_NOT_SAVED_ENTRY_KEPT: &str = "XTrace could not save your membership, and your entry may still be on the leaderboard. Joining again with the same email signs you back in to it.";
/// Shown when the code was right but the nickname is taken; the code stays
/// valid.
const NICKNAME_TAKEN_AT_CODE: &str =
    "That nickname is taken. Choose another one and confirm again.";
/// Shown when the code step was asked for with no code sent in this launch.
const NO_CODE_SENT: &str =
    "No code was sent in this session. Enter your nickname and email to get a new code.";
/// Shown when an upload could not reach the leaderboard.
const UPLOAD_UNREACHABLE: &str =
    "Could not reach the leaderboard. Your number will be sent again within the hour.";

/// The leaderboard's answer to a join: a code was emailed.
#[derive(Deserialize)]
struct CodeSent {
    pending_id: String,
}

/// The leaderboard's answer to a right code.
#[derive(Deserialize)]
struct Joined {
    key: String,
    nickname: String,
    /// The email already had an entry, which this signs back in to.
    rejoined: bool,
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
        Err(Self::refusal(answer.status, &answer.body))
    }

    /// What an answer other than the expected one means.
    fn refusal(status: u16, body: &[u8]) -> Refusal {
        #[derive(Deserialize)]
        struct Error {
            error: String,
            retry_after_seconds: Option<u64>,
        }
        let error = serde_json::from_slice::<Error>(body).ok();
        match (status, error) {
            (429, error) => Refusal::RateLimited(error.and_then(|e| e.retry_after_seconds)),
            (413, _) => Refusal::Status(413),
            (400..=499, Some(error)) => Refusal::Code(error.error),
            // Of the server's own failures, only this one says something the
            // person can act on; the rest are a server error.
            (503, Some(error)) if error.error == "email_unavailable" => Refusal::Code(error.error),
            (status, _) => Refusal::Status(status),
        }
    }

    fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, Refusal> {
        serde_json::from_slice(body).map_err(|_| Refusal::Unreadable)
    }

    /// Asks for a code by email.
    fn join(&self, nickname: &str, email: &str) -> Result<CodeSent, Refusal> {
        let body = self.call(
            Request {
                method: Method::Post,
                path: "/api/join",
                key: None,
                body: Some(json!({ "nickname": nickname, "email": email })),
            },
            202,
        )?;
        let sent: CodeSent = Self::parse(&body)?;
        if sent.pending_id.is_empty() || sent.pending_id.len() > 256 {
            return Err(Refusal::Unreadable);
        }
        Ok(sent)
    }

    /// Gives the emailed code for a pending join, with a new nickname when
    /// the one asked for was taken.
    fn verify(
        &self,
        pending_id: &str,
        code: &str,
        nickname: Option<&str>,
    ) -> Result<Joined, Refusal> {
        let mut body = json!({ "pending_id": pending_id, "code": code });
        if let Some(nickname) = nickname {
            body["nickname"] = json!(nickname);
        }
        let body = self.call(
            Request {
                method: Method::Post,
                path: "/api/join/verify",
                key: None,
                body: Some(body),
            },
            201,
        )?;
        let joined: Joined = Self::parse(&body)?;
        if joined.key.is_empty() || joined.key.len() > 256 || joined.nickname.is_empty() {
            return Err(Refusal::Unreadable);
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
        Self::parse(&body)
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
        Self::parse(&body)
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
    /// Whether the membership `key` names was the saved one, now removed.
    fn forget(&self, key: &str) -> Result<bool, StateError>;
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
    fn forget(&self, key: &str) -> Result<bool, StateError> {
        self.with(|state| state.with_store(|store| Ok(store.leave_leaderboard(key)?)))
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
    /// Whether this copy of the app may talk to the leaderboard.
    pub available: bool,
    /// Why not, in plain words, when it may not.
    pub unavailable_reason: Option<String>,
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
    /// While not joined: a code was emailed in this launch and waits to be
    /// entered.
    pub code_sent: Option<LeaderboardCodeSent>,
    /// Said once, in the answer to the code that signed back in to an
    /// existing entry.
    pub welcome: Option<String>,
}

/// A join waiting for its emailed code, as the page shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct LeaderboardCodeSent {
    /// Where the code went, as typed.
    pub email: String,
    /// The nickname asked for, kept for "Use a different email".
    pub nickname: String,
    /// The code was right but the nickname is taken: the same code is
    /// confirmed again with another nickname.
    pub choose_nickname: bool,
}

/// A join waiting for its code: the leaderboard's id for it and what was
/// typed. Kept in memory only; a restart starts over.
#[derive(Clone)]
struct PendingJoin {
    id: String,
    nickname: String,
    email: String,
    /// The leaderboard said the nickname is taken after a right code.
    nickname_taken: bool,
}

/// In-memory notes for the page; a restart starts with none.
#[derive(Default)]
struct Notes {
    upload_error: Option<String>,
    gone: bool,
    pending: Option<PendingJoin>,
    welcome: Option<String>,
}

/// Wall time in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;
/// Called after a successful join, or after a background upload changed
/// what the page shows.
pub type Hook = Arc<dyn Fn() + Send + Sync>;

/// The leaderboard service of one launch.
pub struct LeaderboardService {
    /// The transport, or why this launch sends nothing ([`BLOCKED_COPY`],
    /// [`NO_CLIENT`]).
    transport: Result<Arc<dyn Transport>, String>,
    host: Arc<dyn Host>,
    clock: Clock,
    on_joined: Hook,
    /// Tells the page to read the status again.
    on_changed: Hook,
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
        transport: Result<Arc<dyn Transport>, String>,
        host: Arc<dyn Host>,
        clock: Clock,
        on_joined: Hook,
        on_changed: Hook,
    ) -> Self {
        Self {
            transport,
            host,
            clock,
            on_joined,
            on_changed,
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
        match &self.transport {
            Ok(transport) => Ok(Api(transport.as_ref())),
            Err(reason) => Err(reason.clone()),
        }
    }

    /// Forgets the membership `key` names after the leaderboard said it no
    /// longer has it; the page says so only when that removed it.
    fn forget_gone(&self, key: &str) {
        let removed = self.host.forget(key).unwrap_or(false);
        let mut notes = lock(&self.notes);
        notes.upload_error = None;
        if removed {
            notes.gone = true;
        }
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
        let mut notes = lock(&self.notes);
        let joined = membership.is_some();
        let welcome = notes.welcome.take();
        LeaderboardStatus {
            available: self.transport.is_ok(),
            unavailable_reason: self.transport.as_ref().err().cloned(),
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
            code_sent: notes.pending.as_ref().filter(|_| !joined).map(|pending| {
                LeaderboardCodeSent {
                    email: pending.email.clone(),
                    nickname: pending.nickname.clone(),
                    choose_nickname: pending.nickname_taken,
                }
            }),
            welcome: welcome.filter(|_| joined),
        }
    }

    /// The first step of joining: asks the leaderboard to email a code for
    /// the nickname and email as typed (it checks them). Asking again sends
    /// a new code; only the newest works.
    pub fn join(
        &self,
        nickname: &str,
        email: &str,
    ) -> Result<LeaderboardStatus, LeaderboardRefusal> {
        let api = self.api()?;
        {
            let _work = lock(&self.work);
            if self
                .host
                .membership()
                .map_err(saved_state_failed)?
                .is_some()
            {
                return Err(String::from("You have already joined the leaderboard.").into());
            }
            let sent = api
                .join(nickname, email)
                .map_err(|refusal| LeaderboardRefusal::from(&refusal))?;
            let mut notes = lock(&self.notes);
            notes.gone = false;
            notes.pending = Some(PendingJoin {
                id: sent.pending_id,
                nickname: nickname.to_owned(),
                email: email.to_owned(),
                nickname_taken: false,
            });
        }
        Ok(self.status())
    }

    /// The second step: gives the emailed code, and a new nickname after the
    /// leaderboard said the first one is taken. A right code saves the key
    /// and nickname the leaderboard answers with, then sends the number. A
    /// new entry tells the telemetry service it joined; signing back in to
    /// an existing entry (the same email, on another Mac or after leaving
    /// this app) does not, as that person already opted in.
    pub fn confirm(
        &self,
        code: &str,
        nickname: Option<&str>,
    ) -> Result<LeaderboardStatus, LeaderboardRefusal> {
        let api = self.api()?;
        let rejoined = {
            let _work = lock(&self.work);
            if self
                .host
                .membership()
                .map_err(saved_state_failed)?
                .is_some()
            {
                return Err(String::from("You have already joined the leaderboard.").into());
            }
            let pending = lock(&self.notes)
                .pending
                .clone()
                .ok_or_else(|| String::from(NO_CODE_SENT))?;
            let joined = match api.verify(&pending.id, code, nickname) {
                Ok(joined) => joined,
                Err(refusal) => {
                    let mut notes = lock(&self.notes);
                    let code_is = |name: &str| matches!(&refusal, Refusal::Code(c) if c == name);
                    if code_is("code_expired") {
                        // The leaderboard deleted this pending join: start over.
                        notes.pending = None;
                    } else if code_is("nickname_taken") {
                        // The code was right and stays valid: the same code
                        // is confirmed again with another nickname.
                        if let Some(pending) = notes.pending.as_mut() {
                            pending.nickname_taken = true;
                            if let Some(nickname) = nickname {
                                pending.nickname = nickname.to_owned();
                            }
                        }
                        return Err(LeaderboardRefusal {
                            message: NICKNAME_TAKEN_AT_CODE.into(),
                            field: Some(LeaderboardField::Nickname),
                        });
                    }
                    return Err(LeaderboardRefusal::from(&refusal));
                }
            };
            // The pending join is gone on the leaderboard either way.
            lock(&self.notes).pending = None;
            let membership = LeaderboardMembership {
                key: joined.key,
                nickname: joined.nickname,
                last_sent: None,
            };
            if let Err(error) = self.host.join(&membership) {
                if joined.rejoined {
                    // An existing entry is never deleted here: it keeps its
                    // number, and the same email signs back in to it.
                    return Err(String::from(JOIN_NOT_SAVED_ENTRY_KEPT).into());
                }
                // Without the saved key the member could never leave, so the
                // new entry is removed again.
                return Err(match api.delete(&membership.key) {
                    Ok(()) => saved_state_failed(error),
                    Err(_) => JOIN_NOT_SAVED_ENTRY_KEPT.into(),
                }
                .into());
            }
            let mut notes = lock(&self.notes);
            *notes = Notes::default();
            notes.welcome = joined.rejoined.then(|| {
                format!(
                    "Welcome back. You are signed in to your existing entry as {}.",
                    membership.nickname
                )
            });
            joined.rejoined
        };
        if !rejoined {
            (self.on_joined)();
        }
        self.send_number();
        Ok(self.status())
    }

    /// "Use a different email": forgets the code waiting to be entered.
    pub fn cancel_code(&self) -> LeaderboardStatus {
        lock(&self.notes).pending = None;
        self.status()
    }

    /// Changes the nickname; the leaderboard checks it.
    pub fn rename(&self, nickname: &str) -> Result<LeaderboardStatus, LeaderboardRefusal> {
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
                Err(refusal) => return Err(LeaderboardRefusal::from(&refusal)),
            }
        }
        Ok(self.status())
    }

    /// Asks the leaderboard to delete the member, then forgets the saved
    /// membership. When the leaderboard cannot be reached nothing is
    /// forgotten, so Leave can be tried again.
    pub fn leave(&self) -> Result<LeaderboardStatus, LeaderboardRefusal> {
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
                    )
                    .into());
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

    /// The background upload: sends the current number when joined, then
    /// tells the page to read the status again. Does nothing when not joined.
    pub fn upload(&self) {
        if self.send_number() {
            (self.on_changed)();
        }
    }

    /// Sends the current number when joined; returns whether anything was
    /// tried. A failure is kept for the page and the next hour tries again.
    fn send_number(&self) -> bool {
        let Ok(api) = self.api() else { return false };
        let _work = lock(&self.work);
        let membership = match self.host.membership() {
            Ok(Some(membership)) => membership,
            Ok(None) => return false,
            Err(error) => {
                lock(&self.notes).upload_error = Some(saved_state_failed(error));
                return true;
            }
        };
        let tenths = match self.host.leverage_tenths() {
            Ok(tenths) => tenths,
            Err(_) => {
                lock(&self.notes).upload_error = Some(NUMBER_UNREADABLE.into());
                return true;
            }
        };
        let failed = match api.put_score(&membership.key, tenths) {
            Ok(()) => {
                let sent = SentScore {
                    leverage_tenths: tenths,
                    at_ms: (self.clock)(),
                };
                self.host
                    .record(&membership.key, sent)
                    .err()
                    .map(|_| RECORD_FAILED.to_owned())
            }
            Err(refusal) if refusal.gone() => {
                self.forget_gone(&membership.key);
                return true;
            }
            Err(Refusal::Unreachable) => Some(UPLOAD_UNREACHABLE.into()),
            Err(Refusal::RateLimited(_)) => Some(UPLOAD_RATE_LIMITED.into()),
            Err(refusal) => Some(refusal_message(&refusal)),
        };
        lock(&self.notes).upload_error = failed;
        true
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
