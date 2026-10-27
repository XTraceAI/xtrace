use super::*;
use crate::telemetry::{Telemetry, consent::Consent};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use xt_metrics::{HumanHours, HumanHoursPeriods};
use xt_store::Store;

const KEY: &str = "k3y_k3y_k3y_k3y_k3y_k3y_k3y_k3y_k3y_k3y_k3y";
const H: u64 = 3_600_000;
const NOW: i64 = 1_800_000_000_000;
const PENDING: &str = "pending-1";
const CODE: &str = "123456";

/// Both steps of joining, as the page does them.
fn join(
    service: &LeaderboardService,
    nickname: &str,
    email: &str,
) -> Result<LeaderboardStatus, LeaderboardRefusal> {
    service.join(nickname, email)?;
    service.confirm(CODE, None)
}

/// The app's saved state over a test database, with a number to send.
struct FakeHost {
    store: Mutex<Store>,
    number: Mutex<Result<Option<u32>, ()>>,
    fail_join: bool,
    fail_record: bool,
    unreadable: std::sync::atomic::AtomicBool,
}

impl FakeHost {
    fn new(number: Option<u32>) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(Store::open_in_memory().unwrap()),
            number: Mutex::new(Ok(number)),
            fail_join: false,
            fail_record: false,
            unreadable: Default::default(),
        })
    }
    fn joined(nickname: &str, number: Option<u32>) -> Arc<Self> {
        let host = Self::new(number);
        lock(&host.store)
            .join_leaderboard(&LeaderboardMembership {
                key: KEY.into(),
                nickname: nickname.into(),
                last_sent: None,
            })
            .unwrap();
        host
    }
    fn saved(&self) -> Option<LeaderboardMembership> {
        lock(&self.store).leaderboard_membership().unwrap()
    }
}

impl Host for FakeHost {
    fn membership(&self) -> Result<Option<LeaderboardMembership>, StateError> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(StateError::Closed);
        }
        Ok(lock(&self.store).leaderboard_membership()?)
    }
    fn join(&self, membership: &LeaderboardMembership) -> Result<(), StateError> {
        if self.fail_join {
            return Err(StateError::Closed);
        }
        Ok(lock(&self.store).join_leaderboard(membership).map(drop)?)
    }
    fn rename(&self, key: &str, nickname: &str) -> Result<(), StateError> {
        Ok(lock(&self.store)
            .rename_leaderboard_member(key, nickname)
            .map(drop)?)
    }
    fn record(&self, key: &str, sent: SentScore) -> Result<(), StateError> {
        if self.fail_record {
            return Err(StateError::Closed);
        }
        Ok(lock(&self.store)
            .record_leaderboard_score(key, sent)
            .map(drop)?)
    }
    fn forget(&self, key: &str) -> Result<bool, StateError> {
        Ok(lock(&self.store).leave_leaderboard(key)?)
    }
    fn leverage_tenths(&self) -> Result<Option<u32>, StateError> {
        lock(&self.number).map_err(|()| StateError::Closed)
    }
}

type Route = Box<dyn Fn(&Request) -> Result<Answer, Unreachable> + Send + Sync>;

/// Records every request and answers it by `route`.
struct FakeTransport {
    seen: Mutex<Vec<Request>>,
    route: Mutex<Route>,
}

impl FakeTransport {
    fn new(
        route: impl Fn(&Request) -> Result<Answer, Unreachable> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            route: Mutex::new(Box::new(route)),
        })
    }
    fn reroute(
        &self,
        route: impl Fn(&Request) -> Result<Answer, Unreachable> + Send + Sync + 'static,
    ) {
        *lock(&self.route) = Box::new(route);
    }
    fn seen(&self) -> Vec<Request> {
        lock(&self.seen).clone()
    }
    fn lines(&self) -> Vec<String> {
        self.seen()
            .iter()
            .map(|r| format!("{:?} {}", r.method, r.path))
            .collect()
    }
}

impl Transport for FakeTransport {
    fn send(&self, request: &Request) -> Result<Answer, Unreachable> {
        lock(&self.seen).push(request.clone());
        (lock(&self.route))(request)
    }
}

fn answer(status: u16, body: Value) -> Result<Answer, Unreachable> {
    Ok(Answer {
        status,
        body: if body.is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(&body).unwrap()
        },
    })
}

fn error(status: u16, code: &str) -> Result<Answer, Unreachable> {
    answer(status, json!({ "error": code }))
}

/// A leaderboard that accepts everything, ranking the member second.
fn accepting(request: &Request) -> Result<Answer, Unreachable> {
    match (request.method, request.path) {
        (Method::Post, "/api/join") => answer(202, json!({ "pending_id": PENDING })),
        (Method::Post, "/api/join/verify") => {
            let body = request.body.as_ref().unwrap();
            if body["pending_id"] != PENDING {
                error(410, "code_expired")
            } else if body["code"] == CODE {
                answer(
                    201,
                    json!({ "key": KEY, "nickname": "ada", "rejoined": false }),
                )
            } else {
                error(400, "invalid_code")
            }
        }
        (Method::Put, "/api/score") => answer(204, Value::Null),
        (Method::Get, "/api/me") => answer(
            200,
            json!({ "nickname": "ada", "leverage_tenths": 123, "listed": true, "rank": 2,
                    "score_updated_at": "2027-01-15T08:00:00Z" }),
        ),
        (Method::Patch, "/api/me") => answer(
            200,
            json!({ "nickname": request.body.as_ref().unwrap()["nickname"] }),
        ),
        (Method::Delete, "/api/me") => answer(204, Value::Null),
        _ => answer(404, json!({ "error": "not_found" })),
    }
}

struct Rig {
    service: LeaderboardService,
    host: Arc<FakeHost>,
    transport: Arc<FakeTransport>,
    joins: Arc<AtomicUsize>,
    changes: Arc<AtomicUsize>,
}

fn setup(host: Arc<FakeHost>, transport: Arc<FakeTransport>) -> Rig {
    let joins = Arc::new(AtomicUsize::new(0));
    let changes = Arc::new(AtomicUsize::new(0));
    let (counted, changed) = (joins.clone(), changes.clone());
    let service = LeaderboardService::new(
        DEFAULT_URL,
        Ok(transport.clone() as Arc<dyn Transport>),
        host.clone(),
        Arc::new(|| NOW),
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
        Arc::new(move || {
            changed.fetch_add(1, Ordering::SeqCst);
        }),
    );
    Rig {
        service,
        host,
        transport,
        joins,
        changes,
    }
}

fn tile(value: Option<f64>) -> MetricTile {
    crate::dashboard_dto::MetricTile {
        value,
        unit: "ratio".into(),
        rule_id: "M-08".into(),
        reason: None,
        note: None,
        current_n: None,
        previous_n: None,
        sample_unit: "sessions".into(),
        delta: crate::dashboard_dto::MetricDelta {
            previous: None,
            pct: None,
            suppressed: true,
        },
    }
}

fn hours(active_ms: Option<u64>, messages: Option<u64>) -> HumanHours {
    HumanHours {
        break_minutes: 60,
        start_ms: 0,
        end_ms: 86_400_000,
        active_ms,
        messages,
        by_day: Vec::new(),
    }
}

#[test]
fn the_number_is_the_leverage_tile_in_the_tenths_the_screen_writes() {
    // Known: 37 agent hours over 3 of yours is 12.333…×, written 12.3×.
    let known = crate::dashboard::leverage_tile(
        (37 * H, 0),
        &HumanHoursPeriods {
            current: hours(Some(3 * H), Some(30)),
            previous: hours(Some(0), Some(0)),
        },
        (Some(4), Some(0), "sessions"),
    )
    .unwrap();
    assert_eq!(leaderboard_tenths(&known), Some(123));
    // Unknown: no human time, so no number.
    let unknown = crate::dashboard::leverage_tile(
        (37 * H, 0),
        &HumanHoursPeriods {
            current: hours(Some(0), Some(0)),
            previous: hours(Some(0), Some(0)),
        },
        (Some(4), Some(0), "sessions"),
    )
    .unwrap();
    assert_eq!(unknown.value, None);
    assert_eq!(leaderboard_tenths(&unknown), None);
    // Rounding edges, as the screen writes them: 1.05 → 1.1×, 0.15 → 0.2×.
    for (value, tenths) in [
        (1.05, 11),
        (1.049_999_999_999_999_8, 10),
        (0.15, 2),
        (0.04, 0),
    ] {
        assert_eq!(
            leaderboard_tenths(&tile(Some(value))),
            Some(tenths),
            "{value}"
        );
        assert_eq!(
            crate::dashboard::written_tenths(value).map(|t| t as u32),
            Some(tenths)
        );
    }
    // 1.05 is the first value the Dashboard no longer marks at or below 1×.
    assert_eq!(leaderboard_tenths(&tile(Some(1e12))), None);
}

#[test]
fn the_address_is_the_default_unless_a_plain_https_or_local_http_origin_is_given() {
    assert_eq!(base_url(None), "https://leaderboard.xtrace.ai");
    for (env, expected) in [
        ("http://127.0.0.1:8787", "http://127.0.0.1:8787"),
        ("http://127.0.0.1:8787/", "http://127.0.0.1:8787"),
        ("http://localhost:3000", "http://localhost:3000"),
        ("http://[::1]:3000", "http://[::1]:3000"),
        (
            " https://staging.example.com ",
            "https://staging.example.com",
        ),
        ("http://example.com", DEFAULT_URL),
        ("https://example.com/path", DEFAULT_URL),
        ("https://user:pw@example.com", DEFAULT_URL),
        ("https://example.com/?q=1", DEFAULT_URL),
        ("ftp://127.0.0.1", DEFAULT_URL),
        ("not a url", DEFAULT_URL),
        ("", DEFAULT_URL),
    ] {
        assert_eq!(base_url(Some(env)), expected, "{env}");
    }
}

#[test]
fn joining_sends_the_nickname_and_email_saves_only_key_and_nickname_then_sends_the_number() {
    let rig = setup(FakeHost::new(Some(123)), FakeTransport::new(accepting));
    // Step one: a code is emailed; nothing is saved yet.
    let waiting = rig.service.join("ada", "ada@example.com").unwrap();
    assert!(!waiting.joined);
    assert_eq!(
        waiting.code_sent,
        Some(LeaderboardCodeSent {
            email: "ada@example.com".into(),
            nickname: "ada".into(),
            choose_nickname: false,
        })
    );
    assert_eq!(rig.host.saved(), None);
    assert_eq!(rig.joins.load(Ordering::SeqCst), 0);
    // Step two: the code.
    let status = rig.service.confirm(CODE, None).unwrap();
    assert_eq!(
        rig.transport.lines(),
        [
            "Post /api/join",
            "Post /api/join/verify",
            "Put /api/score",
            "Get /api/me"
        ]
    );
    let seen = rig.transport.seen();
    assert_eq!(seen[0].key, None);
    assert_eq!(
        seen[0].body,
        Some(json!({ "nickname": "ada", "email": "ada@example.com" }))
    );
    assert_eq!(seen[1].key, None);
    assert_eq!(
        seen[1].body,
        Some(json!({ "pending_id": PENDING, "code": CODE }))
    );
    assert_eq!(seen[2].key.as_deref(), Some(KEY));
    assert_eq!(seen[2].body, Some(json!({ "leverage_tenths": 123 })));
    assert_eq!(seen[3].key.as_deref(), Some(KEY));
    assert_eq!(seen[3].body, None);
    let saved = rig.host.saved().unwrap();
    assert_eq!((saved.key.as_str(), saved.nickname.as_str()), (KEY, "ada"));
    assert_eq!(
        saved.last_sent,
        Some(SentScore {
            leverage_tenths: Some(123),
            at_ms: NOW
        })
    );
    // The email is in no saved field.
    assert!(!format!("{saved:?}").contains("example.com"));
    assert_eq!(rig.joins.load(Ordering::SeqCst), 1);
    // The join's own upload is answered by the join; nothing extra is announced.
    assert_eq!(rig.changes.load(Ordering::SeqCst), 0);
    assert_eq!(
        status,
        LeaderboardStatus {
            available: true,
            unavailable_reason: None,
            joined: true,
            nickname: Some("ada".into()),
            last_sent: Some(LeaderboardSent {
                leverage_tenths: Some(123),
                at_ms: NOW
            }),
            standing: Some(LeaderboardStanding::Ranked { rank: 2 }),
            page_url: "https://leaderboard.xtrace.ai/".into(),
            error: None,
            notice: None,
            code_sent: None,
            welcome: None,
        }
    );
    // A second join is refused before anything is sent.
    assert!(rig.service.join("bob", "b@example.com").is_err());
    assert!(rig.service.confirm(CODE, None).is_err());
    assert_eq!(rig.transport.seen().len(), 4);
}

#[test]
fn an_unknown_number_is_sent_as_null_and_the_page_says_why_it_is_not_listed() {
    let transport = FakeTransport::new(|request| match (request.method, request.path) {
        (Method::Get, "/api/me") => answer(
            200,
            json!({ "nickname": "ada", "leverage_tenths": null, "listed": false, "rank": null,
                    "score_updated_at": "2027-01-15T08:00:00Z" }),
        ),
        _ => accepting(request),
    });
    let rig = setup(FakeHost::joined("ada", None), transport);
    rig.service.upload();
    assert_eq!(
        rig.transport.seen()[0].body,
        Some(json!({ "leverage_tenths": null }))
    );
    let status = rig.service.status();
    assert_eq!(
        status.last_sent,
        Some(LeaderboardSent {
            leverage_tenths: None,
            at_ms: NOW
        })
    );
    assert_eq!(
        status.standing,
        Some(LeaderboardStanding::NotListed {
            reason: "Not listed yet: there is no leverage number for your last 7 days.".into()
        })
    );
}

#[test]
fn a_join_the_leaderboard_refuses_saves_nothing_and_says_why() {
    for (code, message) in [
        ("nickname_taken", "That nickname is taken. Try another one."),
        (
            "invalid_nickname",
            "Nicknames are 3 to 20 characters: letters, digits, _ and - only.",
        ),
        ("invalid_email", "That email address does not look right."),
        (
            "invalid_request",
            "The leaderboard did not accept the request.",
        ),
    ] {
        let status = if code == "nickname_taken" { 409 } else { 400 };
        let rig = setup(
            FakeHost::new(Some(1)),
            FakeTransport::new(move |_| error(status, code)),
        );
        let refused = rig.service.join("x", "y").unwrap_err();
        assert_eq!(refused.message, message, "{code}");
        let field = match code {
            "nickname_taken" | "invalid_nickname" => Some(LeaderboardField::Nickname),
            "invalid_email" => Some(LeaderboardField::Email),
            _ => None,
        };
        assert_eq!(refused.field, field, "{code}");
        assert_eq!(rig.host.saved(), None);
        assert_eq!(rig.transport.seen().len(), 1);
        assert_eq!(rig.joins.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn every_refusal_has_plain_words() {
    for (refusal, message) in [
        (
            Refusal::Unreachable,
            "Could not reach the leaderboard. Check your internet connection and try again.",
        ),
        (
            Refusal::Code("nickname_taken".into()),
            "That nickname is taken. Try another one.",
        ),
        (
            Refusal::Code("invalid_leverage".into()),
            "The leaderboard did not accept your number.",
        ),
        (Refusal::Code("unknown_key".into()), GONE),
        (
            Refusal::Code("something_new".into()),
            "The leaderboard did not accept the request.",
        ),
        (Refusal::Status(413), "That nickname or email is too long."),
        (
            Refusal::Status(500),
            "The leaderboard answered with an error (500). Try again later.",
        ),
    ] {
        assert_eq!(refusal_message(&refusal), message);
    }
    // A 413 is too long whatever its body says; a 5xx body is not a code.
    let rig = setup(
        FakeHost::new(None),
        FakeTransport::new(|_| error(413, "invalid_request")),
    );
    assert_eq!(
        rig.service
            .join("a", &"b".repeat(5000))
            .unwrap_err()
            .message,
        "That nickname or email is too long."
    );
    let rig = setup(
        FakeHost::new(None),
        FakeTransport::new(|_| error(503, "nickname_taken")),
    );
    assert_eq!(
        rig.service.join("a", "b").unwrap_err().message,
        "The leaderboard answered with an error (503). Try again later."
    );
}

#[test]
fn renaming_sends_the_new_nickname_and_saves_the_one_the_leaderboard_kept() {
    let rig = setup(
        FakeHost::joined("ada", Some(5)),
        FakeTransport::new(accepting),
    );
    let status = rig.service.rename("Ada_Lovelace").unwrap();
    let seen = rig.transport.seen();
    assert_eq!(rig.transport.lines(), ["Patch /api/me", "Get /api/me"]);
    assert_eq!(seen[0].key.as_deref(), Some(KEY));
    assert_eq!(seen[0].body, Some(json!({ "nickname": "Ada_Lovelace" })));
    assert_eq!(status.nickname.as_deref(), Some("Ada_Lovelace"));
    assert_eq!(rig.host.saved().unwrap().nickname, "Ada_Lovelace");
    rig.transport.reroute(|_| error(409, "nickname_taken"));
    assert_eq!(
        rig.service.rename("bob").unwrap_err().message,
        "That nickname is taken. Try another one."
    );
    assert_eq!(rig.host.saved().unwrap().nickname, "Ada_Lovelace");
}

#[test]
fn leaving_deletes_on_the_leaderboard_first_and_keeps_the_membership_when_unreachable() {
    let rig = setup(
        FakeHost::joined("ada", Some(5)),
        FakeTransport::new(|_| Err(Unreachable)),
    );
    let refused = rig.service.leave().unwrap_err().message;
    assert_eq!(
        refused,
        "Could not reach the leaderboard. Check your internet connection and try again. You are still on the leaderboard."
    );
    assert!(rig.host.saved().is_some());
    rig.transport.reroute(accepting);
    let status = rig.service.leave().unwrap();
    let seen = rig.transport.seen();
    assert_eq!(seen[1].method, Method::Delete);
    assert_eq!(seen[1].path, "/api/me");
    assert_eq!(seen[1].key.as_deref(), Some(KEY));
    assert_eq!(rig.host.saved(), None);
    assert!(!status.joined && status.notice.is_none() && status.standing.is_none());
    // Not joined: nothing more is asked, and uploads send nothing.
    rig.service.upload();
    assert_eq!(rig.transport.seen().len(), 2);
    assert!(rig.service.leave().is_err());
}

#[test]
fn an_unknown_key_from_any_call_forgets_the_membership_and_says_so() {
    type Call = fn(&LeaderboardService);
    let calls: [(&str, Call); 4] = [
        ("upload", |s| s.upload()),
        ("status", |s| drop(s.status())),
        ("rename", |s| drop(s.rename("bob"))),
        ("leave", |s| drop(s.leave())),
    ];
    for (name, call) in calls {
        let rig = setup(
            FakeHost::joined("ada", Some(5)),
            FakeTransport::new(|_| error(401, "unknown_key")),
        );
        call(&rig.service);
        assert_eq!(rig.host.saved(), None, "{name}");
        let status = rig.service.status();
        assert!(!status.joined, "{name}");
        let expected = (name != "leave").then_some(GONE);
        assert_eq!(status.notice.as_deref(), expected, "{name}");
        // Joining again clears the note.
        rig.transport.reroute(accepting);
        let joined = join(&rig.service, "ada", "a@example.com").unwrap();
        assert!(joined.joined && joined.notice.is_none(), "{name}");
    }
    // A 401 that is not the leaderboard's own answer forgets nothing.
    let rig = setup(
        FakeHost::joined("ada", Some(5)),
        FakeTransport::new(|_| answer(401, Value::Null)),
    );
    rig.service.upload();
    assert!(rig.host.saved().is_some());
}

#[test]
fn a_failed_upload_is_kept_for_the_page_and_the_next_one_tries_again() {
    let rig = setup(
        FakeHost::joined("ada", Some(77)),
        FakeTransport::new(|_| Err(Unreachable)),
    );
    rig.service.upload();
    assert_eq!(rig.host.saved().unwrap().last_sent, None);
    let status = rig.service.status();
    assert!(status.joined);
    assert_eq!(status.standing, Some(LeaderboardStanding::Unknown));
    assert!(status.error.is_some());
    rig.transport.reroute(accepting);
    // The status read reaches the leaderboard, but the upload has not: it says so.
    assert_eq!(
        rig.service.status().error.as_deref(),
        Some("Could not reach the leaderboard. Your number will be sent again within the hour.")
    );
    rig.service.upload();
    let status = rig.service.status();
    assert_eq!(status.error, None);
    assert_eq!(status.last_sent.unwrap().leverage_tenths, Some(77));
    // A number that cannot be read sends nothing and says so.
    *lock(&rig.host.number) = Err(());
    let before = rig.transport.seen().len();
    rig.service.upload();
    assert_eq!(rig.transport.seen().len(), before);
    assert_eq!(
        rig.service.status().error.as_deref(),
        Some("Your 7-day leverage could not be read. It will be tried again within the hour.")
    );
}

#[test]
fn a_join_whose_key_cannot_be_saved_is_deleted_again() {
    let host = Arc::new(FakeHost {
        store: Mutex::new(Store::open_in_memory().unwrap()),
        number: Mutex::new(Ok(Some(1))),
        fail_join: true,
        fail_record: false,
        unreadable: Default::default(),
    });
    let rig = setup(host, FakeTransport::new(accepting));
    assert_eq!(
        join(&rig.service, "ada", "a@example.com")
            .unwrap_err()
            .message,
        "The leaderboard membership saved in this app could not be read or changed."
    );
    assert_eq!(
        rig.transport.lines(),
        ["Post /api/join", "Post /api/join/verify", "Delete /api/me"]
    );
    assert_eq!(rig.transport.seen()[2].key.as_deref(), Some(KEY));
    assert_eq!(rig.joins.load(Ordering::SeqCst), 0);
    // When that delete fails too, the person is told the entry may remain.
    rig.transport
        .reroute(|request| match (request.method, request.path) {
            (Method::Delete, _) => Err(Unreachable),
            _ => accepting(request),
        });
    assert_eq!(
        join(&rig.service, "ada", "a@example.com")
            .unwrap_err()
            .message,
        "XTrace could not save your membership, and your entry may still be on the leaderboard. Joining again with the same email signs you back in to it."
    );
}

/// Uploads never read the telemetry switch: with telemetry off they still
/// go, and a join only tells the telemetry service, which stays off.
#[test]
fn uploads_do_not_depend_on_the_telemetry_switch() {
    let off = Consent {
        preference_on: false,
        disclosure_shown: true,
    };
    let telemetry = Arc::new(Telemetry::without_sender(off, false));
    assert!(!telemetry.state().can_send);
    let host = FakeHost::new(Some(42));
    let transport = FakeTransport::new(accepting);
    let told = telemetry.clone();
    let service = LeaderboardService::new(
        DEFAULT_URL,
        Ok(transport.clone() as Arc<dyn Transport>),
        host.clone(),
        Arc::new(|| NOW),
        Arc::new(move || told.leaderboard_opted_in()),
        Arc::new(|| {}),
    );
    join(&service, "ada", "a@example.com").unwrap();
    service.upload();
    service.upload();
    let puts = transport
        .seen()
        .iter()
        .filter(|r| r.path == "/api/score")
        .count();
    assert_eq!(puts, 3);
    assert_eq!(
        host.saved().unwrap().last_sent.unwrap().leverage_tenths,
        Some(42)
    );
    assert!(!telemetry.state().can_send && !telemetry.state().preference_on);
}

/// Only the installed app on its own data sends to the real leaderboard; any
/// copy may use a server on this Mac. The rule is telemetry's own.
#[test]
fn only_the_installed_app_on_its_own_data_or_a_local_server_may_send() {
    use crate::telemetry::{INSTALLED_IDENTIFIER, installed_own_data};
    assert!(installed_own_data(true, INSTALLED_IDENTIFIER));
    for (own, id) in [
        (false, INSTALLED_IDENTIFIER),
        (true, "ai.xtrace.app.test"),
        (true, "ai.xtrace.desktop"),
    ] {
        assert!(!installed_own_data(own, id), "{own} {id}");
    }
    assert!(may_send(true, DEFAULT_URL));
    assert!(!may_send(false, DEFAULT_URL));
    assert!(!may_send(false, "https://staging.example.com"));
    for local in [
        "http://127.0.0.1:8787",
        "http://localhost:3000",
        "http://[::1]:3000",
    ] {
        assert!(may_send(false, &base_url(Some(local))), "{local}");
    }
}

/// A copy that may not send keeps its saved membership and says why, and
/// sends nothing: not the upload, not the status read, not a change.
#[test]
fn a_blocked_copy_sends_nothing_and_says_so() {
    for reason in [BLOCKED_COPY, NO_CLIENT] {
        let host = FakeHost::joined("ada", Some(1));
        let blocked = LeaderboardService::new(
            DEFAULT_URL,
            Err(reason.to_owned()),
            host.clone(),
            Arc::new(|| NOW),
            Arc::new(|| {}),
            Arc::new(|| panic!("nothing changed")),
        );
        blocked.upload();
        let status = blocked.status();
        assert!(!status.available);
        assert_eq!(status.unavailable_reason.as_deref(), Some(reason));
        assert_eq!(blocked.join("a", "b").unwrap_err().message, reason);
        assert_eq!(blocked.confirm(CODE, None).unwrap_err().message, reason);
        assert_eq!(blocked.rename("b").unwrap_err().message, reason);
        assert_eq!(blocked.leave().unwrap_err().message, reason);
        assert!(host.saved().is_some());
    }
    assert_eq!(
        BLOCKED_COPY,
        "This copy of XTrace does not send to the leaderboard."
    );
}

/// The production transport against a local server: method, path, bearer
/// header and JSON body, and the server's answer read back.
#[test]
fn the_http_transport_sends_the_contract_request_shapes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(
        Vec::<(String, Vec<(String, String)>, String)>::new(),
    ));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut headers = Vec::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
                let (name, value) = header.trim().split_once(':').unwrap();
                headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
            }
            let length = headers
                .iter()
                .find(|(name, _)| name == "content-length")
                .map_or(0, |(_, value)| value.parse::<usize>().unwrap());
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request = line.trim().to_owned();
            let (status, reply) = if request.starts_with("PUT") {
                ("401 Unauthorized", r#"{"error":"unknown_key"}"#)
            } else if request.starts_with("POST /api/join/verify") {
                (
                    "201 Created",
                    r#"{"key":"abc","nickname":"ada","rejoined":false}"#,
                )
            } else if request.starts_with("GET") {
                (
                    "429 Too Many Requests",
                    r#"{"error":"rate_limited","retry_after_seconds":61}"#,
                )
            } else {
                ("202 Accepted", r#"{"pending_id":"p1"}"#)
            };
            lock(&log).push((request, headers, String::from_utf8(body).unwrap()));
            let mut stream = stream;
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .as_bytes(),
            );
        }
    });
    let transport = HttpTransport::new(&base_url(Some(&base))).unwrap();
    let api = Api(&transport);
    assert_eq!(api.join("ada", "ada@example.com").unwrap().pending_id, "p1");
    let joined = api.verify("p1", "123456", None).unwrap();
    assert_eq!(
        (
            joined.key.as_str(),
            joined.nickname.as_str(),
            joined.rejoined
        ),
        ("abc", "ada", false)
    );
    let refused = api.put_score("abc", Some(123)).unwrap_err();
    assert!(refused.gone());
    assert_eq!(api.me("abc").err(), Some(Refusal::RateLimited(Some(61))));
    let seen = lock(&seen).clone();
    assert_eq!(seen[0].0, "POST /api/join HTTP/1.1");
    let header = |n: usize, name: &str| {
        seen[n]
            .1
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(header(0, "authorization"), None);
    assert_eq!(
        header(0, "content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&seen[0].2).unwrap(),
        json!({ "nickname": "ada", "email": "ada@example.com" })
    );
    assert_eq!(seen[1].0, "POST /api/join/verify HTTP/1.1");
    assert_eq!(header(1, "authorization"), None);
    assert_eq!(
        serde_json::from_str::<Value>(&seen[1].2).unwrap(),
        json!({ "pending_id": "p1", "code": "123456" })
    );
    assert_eq!(seen[2].0, "PUT /api/score HTTP/1.1");
    assert_eq!(header(2, "authorization").as_deref(), Some("Bearer abc"));
    assert_eq!(
        serde_json::from_str::<Value>(&seen[2].2).unwrap(),
        json!({ "leverage_tenths": 123 })
    );
    assert!(
        header(2, "user-agent")
            .unwrap()
            .starts_with("XTrace-Desktop/")
    );
    // Nothing listening: unreachable, not an answer.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let transport = HttpTransport::new(&address).unwrap();
    assert_eq!(Api(&transport).me("abc").err(), Some(Refusal::Unreachable));
}

#[cfg(target_os = "macos")]
#[test]
fn the_page_opens_only_the_configured_leaderboard() {
    let service = setup(FakeHost::new(None), FakeTransport::new(accepting)).service;
    let command = page_command(service.page_url()).unwrap();
    assert_eq!(command.get_program(), "/usr/bin/open");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args, ["https://leaderboard.xtrace.ai/"]);
}

/// A deliberate Leave, or a stale answer for a membership already gone,
/// never says the leaderboard lost the membership.
#[test]
fn only_a_membership_the_app_still_had_is_reported_lost() {
    let rig = setup(
        FakeHost::joined("ada", Some(5)),
        FakeTransport::new(accepting),
    );
    rig.service.leave().unwrap();
    rig.service.forget_gone(KEY);
    assert_eq!(rig.service.status().notice, None);
}

#[test]
fn background_uploads_tell_the_page_and_error_paths_are_shown() {
    let rig = setup(FakeHost::new(Some(5)), FakeTransport::new(accepting));
    // Not joined: nothing is tried, nothing announced.
    rig.service.upload();
    assert_eq!(rig.changes.load(Ordering::SeqCst), 0);
    join(&rig.service, "ada", "a@example.com").unwrap();
    rig.service.upload();
    assert_eq!(rig.changes.load(Ordering::SeqCst), 1);
    // A saved membership that cannot be read is shown, and announced.
    rig.host.unreadable.store(true, Ordering::SeqCst);
    rig.service.upload();
    assert_eq!(rig.changes.load(Ordering::SeqCst), 2);
    rig.host.unreadable.store(false, Ordering::SeqCst);
    assert_eq!(
        rig.service.status().error.as_deref(),
        Some("The leaderboard membership saved in this app could not be read or changed.")
    );
    // A number sent but not recorded is shown.
    let host = Arc::new(FakeHost {
        store: Mutex::new(Store::open_in_memory().unwrap()),
        number: Mutex::new(Ok(Some(9))),
        fail_join: false,
        fail_record: true,
        unreadable: Default::default(),
    });
    lock(&host.store)
        .join_leaderboard(&LeaderboardMembership {
            key: KEY.into(),
            nickname: "ada".into(),
            last_sent: None,
        })
        .unwrap();
    let rig = setup(host, FakeTransport::new(accepting));
    rig.service.upload();
    assert_eq!(
        rig.service.status().error.as_deref(),
        Some("Your number was sent, but XTrace could not save when it was sent.")
    );
}

#[test]
fn a_join_answer_that_cannot_be_read_says_so() {
    let unreadable = "The leaderboard answered in a way XTrace could not read.";
    // The code step's answer.
    for body in [
        json!({ "nope": 1 }),
        json!({ "key": "", "nickname": "ada", "rejoined": false }),
        json!({ "key": KEY, "nickname": "ada" }),
    ] {
        let rig = setup(
            FakeHost::new(None),
            FakeTransport::new(move |request| match request.path {
                "/api/join/verify" => answer(201, body.clone()),
                _ => accepting(request),
            }),
        );
        assert_eq!(
            join(&rig.service, "ada", "a@example.com")
                .unwrap_err()
                .message,
            unreadable
        );
        assert_eq!(rig.host.saved(), None);
    }
    // The first step's answer.
    for body in [json!({}), json!({ "pending_id": "" })] {
        let rig = setup(
            FakeHost::new(None),
            FakeTransport::new(move |_| answer(202, body.clone())),
        );
        assert_eq!(
            rig.service
                .join("ada", "a@example.com")
                .unwrap_err()
                .message,
            unreadable
        );
        assert_eq!(rig.service.status().code_sent, None);
    }
}

#[test]
fn a_request_never_shows_its_key_or_email_when_printed() {
    let request = Request {
        method: Method::Post,
        path: "/api/join",
        key: Some(KEY.into()),
        body: Some(json!({ "nickname": "ada", "email": "ada@example.com" })),
    };
    let text = format!("{request:?}");
    assert!(!text.contains(KEY) && !text.contains("example.com") && !text.contains("ada\""));
    assert!(text.contains("/api/join") && text.contains("email"));
}

#[test]
fn every_code_step_answer_has_plain_words() {
    for (status, code, message, field) in [
        (
            400,
            "invalid_code",
            "That code is not right. Check the email and try again.",
            Some(LeaderboardField::Code),
        ),
        (
            410,
            "code_expired",
            "That code has expired or was tried too many times. Send a new code.",
            None,
        ),
        (
            503,
            "email_unavailable",
            "The leaderboard can't send emails right now. Try again later.",
            None,
        ),
        (
            503,
            "server_misconfigured",
            "The leaderboard answered with an error (503). Try again later.",
            None,
        ),
    ] {
        let refused = LeaderboardRefusal::from(&Api::refusal(
            status,
            &serde_json::to_vec(&json!({ "error": code })).unwrap(),
        ));
        assert_eq!(refused.message, message, "{code}");
        assert_eq!(refused.field, field, "{code}");
    }
    for (seconds, message) in [
        (Some(1), "Too many tries. Try again in about 1 minute."),
        (Some(60), "Too many tries. Try again in about 1 minute."),
        (Some(61), "Too many tries. Try again in about 2 minutes."),
        (Some(3600), "Too many tries. Try again in about 60 minutes."),
        (Some(0), "Too many tries. Try again in about 1 minute."),
        (None, "Too many tries. Try again later."),
    ] {
        assert_eq!(
            refusal_message(&Refusal::RateLimited(seconds)),
            message,
            "{seconds:?}"
        );
    }
}

/// A wrong code keeps the code step; a code the leaderboard has thrown away
/// (expired, or too many tries) starts over.
#[test]
fn the_code_step_keeps_or_ends_the_pending_join_as_the_leaderboard_does() {
    let rig = setup(FakeHost::new(Some(1)), FakeTransport::new(accepting));
    assert_eq!(
        rig.service.confirm(CODE, None).unwrap_err().message,
        "No code was sent in this session. Enter your nickname and email to get a new code."
    );
    rig.service.join("ada", "a@example.com").unwrap();
    let wrong = rig.service.confirm("000000", None).unwrap_err();
    assert_eq!(wrong.field, Some(LeaderboardField::Code));
    assert!(rig.service.status().code_sent.is_some());
    rig.transport.reroute(accepting);
    rig.service.join("ada", "a@example.com").unwrap();
    rig.transport.reroute(|_| error(410, "code_expired"));
    assert!(rig.service.confirm(CODE, None).is_err());
    assert_eq!(rig.service.status().code_sent, None);
    // "Use a different email" forgets it too, keeping nothing saved.
    rig.transport.reroute(accepting);
    rig.service.join("ada", "a@example.com").unwrap();
    assert_eq!(rig.service.cancel_code().code_sent, None);
    assert!(rig.service.confirm(CODE, None).is_err());
    // A rate-limited new code keeps the one already sent.
    rig.service.join("ada", "a@example.com").unwrap();
    rig.transport.reroute(|_| {
        answer(
            429,
            json!({ "error": "rate_limited", "retry_after_seconds": 120 }),
        )
    });
    assert_eq!(
        rig.service
            .join("ada", "a@example.com")
            .unwrap_err()
            .message,
        "Too many tries. Try again in about 2 minutes."
    );
    assert!(rig.service.status().code_sent.is_some());
    // Success clears it.
    rig.transport.reroute(accepting);
    let joined = rig.service.confirm(CODE, None).unwrap();
    assert!(joined.joined && joined.code_sent.is_none());
    assert_eq!(rig.service.status().code_sent, None);
}

/// The same email on another Mac signs back in to its entry: saved like a
/// join, welcomed once, and no `leaderboard_opted_in` (that person already
/// opted in).
#[test]
fn signing_back_in_saves_the_entry_welcomes_once_and_is_not_a_new_opt_in() {
    let transport = FakeTransport::new(|request| match request.path {
        "/api/join/verify" => answer(
            201,
            json!({ "key": KEY, "nickname": "Ada_Old", "rejoined": true }),
        ),
        _ => accepting(request),
    });
    let rig = setup(FakeHost::new(Some(7)), transport);
    let status = join(&rig.service, "ignored", "a@example.com").unwrap();
    assert_eq!(rig.host.saved().unwrap().nickname, "Ada_Old");
    assert_eq!(
        status.welcome.as_deref(),
        Some("Welcome back. You are signed in to your existing entry as Ada_Old.")
    );
    assert_eq!(rig.service.status().welcome, None);
    assert_eq!(rig.joins.load(Ordering::SeqCst), 0);
    // A rejoin whose key cannot be saved never deletes the existing entry.
    let host = Arc::new(FakeHost {
        store: Mutex::new(Store::open_in_memory().unwrap()),
        number: Mutex::new(Ok(Some(1))),
        fail_join: true,
        fail_record: false,
        unreadable: Default::default(),
    });
    let rig = setup(host, rig.transport.clone());
    let before = rig.transport.seen().len();
    assert!(join(&rig.service, "x", "a@example.com").is_err());
    assert!(
        rig.transport.seen()[before..]
            .iter()
            .all(|request| request.method != Method::Delete)
    );
}

#[test]
fn a_rate_limited_hourly_upload_is_a_failed_upload() {
    let rig = setup(
        FakeHost::joined("ada", Some(3)),
        FakeTransport::new(|request| match request.path {
            "/api/score" => answer(
                429,
                json!({ "error": "rate_limited", "retry_after_seconds": 30 }),
            ),
            _ => accepting(request),
        }),
    );
    rig.service.upload();
    assert_eq!(rig.host.saved().unwrap().last_sent, None);
    assert_eq!(
        rig.service.status().error.as_deref(),
        Some(
            "The leaderboard asked XTrace to wait. Your number will be sent again within the hour."
        )
    );
    rig.transport.reroute(accepting);
    rig.service.upload();
    assert_eq!(rig.service.status().error, None);
}

/// A right code with a taken nickname keeps the code step: the same code is
/// sent again with another nickname, which the leaderboard then saves.
#[test]
fn a_taken_nickname_at_the_code_step_keeps_the_code_for_another_nickname() {
    let transport = FakeTransport::new(|request| match request.path {
        "/api/join/verify" => {
            let body = request.body.as_ref().unwrap();
            match body.get("nickname").and_then(Value::as_str) {
                None | Some("taken_too") => error(409, "nickname_taken"),
                Some(nickname) => answer(
                    201,
                    json!({ "key": KEY, "nickname": nickname, "rejoined": false }),
                ),
            }
        }
        _ => accepting(request),
    });
    let rig = setup(FakeHost::new(Some(4)), transport);
    rig.service.join("ada", "a@example.com").unwrap();
    let refused = rig.service.confirm(CODE, None).unwrap_err();
    assert_eq!(
        refused.message,
        "That nickname is taken. Choose another one and confirm again."
    );
    assert_eq!(refused.field, Some(LeaderboardField::Nickname));
    let waiting = rig.service.status().code_sent.unwrap();
    assert!(waiting.choose_nickname);
    assert_eq!(waiting.nickname, "ada");
    // Another taken one: still waiting, now showing the newest nickname.
    assert!(rig.service.confirm(CODE, Some("taken_too")).is_err());
    assert_eq!(
        rig.service.status().code_sent.unwrap().nickname,
        "taken_too"
    );
    let joined = rig.service.confirm(CODE, Some("ada_2")).unwrap();
    assert!(joined.joined && joined.code_sent.is_none());
    assert_eq!(rig.host.saved().unwrap().nickname, "ada_2");
    let verifies: Vec<_> = rig
        .transport
        .seen()
        .into_iter()
        .filter(|request| request.path == "/api/join/verify")
        .map(|request| request.body.unwrap())
        .collect();
    assert_eq!(
        verifies,
        [
            json!({ "pending_id": PENDING, "code": CODE }),
            json!({ "pending_id": PENDING, "code": CODE, "nickname": "taken_too" }),
            json!({ "pending_id": PENDING, "code": CODE, "nickname": "ada_2" }),
        ]
    );
    assert_eq!(rig.joins.load(Ordering::SeqCst), 1);
}
