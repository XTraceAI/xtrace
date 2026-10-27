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

/// The app's saved state over a test database, with a number to send.
struct FakeHost {
    store: Mutex<Store>,
    number: Mutex<Result<Option<u32>, ()>>,
    fail_join: bool,
}

impl FakeHost {
    fn new(number: Option<u32>) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(Store::open_in_memory().unwrap()),
            number: Mutex::new(Ok(number)),
            fail_join: false,
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
        Ok(lock(&self.store)
            .record_leaderboard_score(key, sent)
            .map(drop)?)
    }
    fn forget(&self, key: &str) -> Result<(), StateError> {
        Ok(lock(&self.store).leave_leaderboard(key).map(drop)?)
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
        (Method::Post, "/api/join") => answer(
            201,
            json!({ "key": KEY, "nickname": request.body.as_ref().unwrap()["nickname"] }),
        ),
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
}

fn setup(host: Arc<FakeHost>, transport: Arc<FakeTransport>) -> Rig {
    let joins = Arc::new(AtomicUsize::new(0));
    let counted = joins.clone();
    let service = LeaderboardService::new(
        DEFAULT_URL,
        Some(transport.clone() as Arc<dyn Transport>),
        host.clone(),
        Arc::new(|| NOW),
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    );
    Rig {
        service,
        host,
        transport,
        joins,
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
    let status = rig.service.join("ada", "ada@example.com").unwrap();
    assert_eq!(
        rig.transport.lines(),
        ["Post /api/join", "Put /api/score", "Get /api/me"]
    );
    let seen = rig.transport.seen();
    assert_eq!(seen[0].key, None);
    assert_eq!(
        seen[0].body,
        Some(json!({ "nickname": "ada", "email": "ada@example.com" }))
    );
    assert_eq!(seen[1].key.as_deref(), Some(KEY));
    assert_eq!(seen[1].body, Some(json!({ "leverage_tenths": 123 })));
    assert_eq!(seen[2].key.as_deref(), Some(KEY));
    assert_eq!(seen[2].body, None);
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
    assert_eq!(
        status,
        LeaderboardStatus {
            available: true,
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
        }
    );
    // A second join is refused before anything is sent.
    assert!(rig.service.join("bob", "b@example.com").is_err());
    assert_eq!(rig.transport.seen().len(), 3);
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
        assert_eq!(rig.service.join("x", "y").unwrap_err(), message, "{code}");
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
        rig.service.join("a", &"b".repeat(5000)).unwrap_err(),
        "That nickname or email is too long."
    );
    let rig = setup(
        FakeHost::new(None),
        FakeTransport::new(|_| error(503, "nickname_taken")),
    );
    assert_eq!(
        rig.service.join("a", "b").unwrap_err(),
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
        rig.service.rename("bob").unwrap_err(),
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
    let refused = rig.service.leave().unwrap_err();
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
        if name != "leave" {
            assert_eq!(status.notice.as_deref(), Some(GONE), "{name}");
        }
        // Joining again clears the note.
        rig.transport.reroute(accepting);
        let joined = rig.service.join("ada", "a@example.com").unwrap();
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
    });
    let rig = setup(host, FakeTransport::new(accepting));
    assert!(rig.service.join("ada", "a@example.com").is_err());
    assert_eq!(rig.transport.lines(), ["Post /api/join", "Delete /api/me"]);
    assert_eq!(rig.transport.seen()[1].key.as_deref(), Some(KEY));
    assert_eq!(rig.joins.load(Ordering::SeqCst), 0);
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
        Some(transport.clone() as Arc<dyn Transport>),
        host.clone(),
        Arc::new(|| NOW),
        Arc::new(move || told.leaderboard_opted_in()),
    );
    service.join("ada", "a@example.com").unwrap();
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
    // And a fixture launch, with no transport, sends nothing at all.
    let unavailable = LeaderboardService::new(
        DEFAULT_URL,
        None,
        FakeHost::joined("ada", Some(1)),
        Arc::new(|| NOW),
        Arc::new(|| {}),
    );
    unavailable.upload();
    assert!(!unavailable.status().available);
    assert!(unavailable.join("a", "b").is_err());
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
            } else {
                ("201 Created", r#"{"key":"abc","nickname":"ada"}"#)
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
    let joined = api.join("ada", "ada@example.com").unwrap();
    assert_eq!(
        (joined.key.as_str(), joined.nickname.as_str()),
        ("abc", "ada")
    );
    let refused = api.put_score("abc", Some(123)).unwrap_err();
    assert!(refused.gone());
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
    assert_eq!(seen[1].0, "PUT /api/score HTTP/1.1");
    assert_eq!(header(1, "authorization").as_deref(), Some("Bearer abc"));
    assert_eq!(
        serde_json::from_str::<Value>(&seen[1].2).unwrap(),
        json!({ "leverage_tenths": 123 })
    );
    assert!(
        header(1, "user-agent")
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
