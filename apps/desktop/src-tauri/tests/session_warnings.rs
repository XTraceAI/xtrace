//! Native warning answers and commands over disposable synthetic databases.
//! Warnings, counts, dismissal growth, dates and message targets share one answer.
use jiff::Timestamp;
use serde_json::{Value, json};
use std::path::Path;
use xt_store::{CanonicalRecord, SessionMeta, SessionSource, Store, retention::RetentionMode};
use xtrace_desktop::dto::{
    DashboardWarnings, SessionWarningKind, SessionWarningsAnswer, SessionWarningsList,
};
use xtrace_desktop::session_warnings_dto::{session_warnings, validate_ids, window_warnings};

/// F1's pinned instant, so the Shell's F1 window and these answers agree.
const NOW: &str = "2026-09-08T00:00:00Z";

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}
fn record(value: Value) -> CanonicalRecord {
    serde_json::from_value(value).unwrap()
}
fn says(uuid: &str, time: &str, text: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[{"type":"text","text":text}]}}))
}
fn reply(uuid: &str, time: &str) -> CanonicalRecord {
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","model":"claude-synthetic",
            "content":[{"type":"text","text":"Synthetic reply"}],
            "usage":{"input_tokens":10,"output_tokens":40,
                     "cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}))
}
fn calls(uuid: &str, time: &str, calls: &[(&str, &str, Value)]) -> CanonicalRecord {
    let content: Vec<Value> = calls
        .iter()
        .map(|(id, name, input)| json!({"type":"tool_use","id":id,"name":name,"input":input}))
        .collect();
    record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
        "message":{"role":"assistant","model":"claude-synthetic","content":content,
            "usage":{"input_tokens":10,"output_tokens":20,
                     "cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}))
}
fn result(uuid: &str, time: &str, call: &str, failed: Option<bool>, text: &str) -> CanonicalRecord {
    let mut block = json!({"type":"tool_result","tool_use_id":call,"content":text});
    if let Some(failed) = failed {
        block["is_error"] = failed.into();
    }
    record(json!({"uuid":uuid,"type":"user","timestamp":time,
        "message":{"role":"user","content":[block]}}))
}

fn session(
    store: &mut Store,
    id: &str,
    host: &str,
    title: &str,
    rows: &[CanonicalRecord],
    keep: bool,
) {
    let mut meta = SessionMeta::new(id, host, SessionSource::Transcript);
    meta.native_session_id = Some(id.into());
    meta.surface = Some("cli".into());
    meta.title = Some(title.into());
    meta.cwd = Some("/work/atlas".into());
    meta.git_branch = Some("main".into());
    meta.started_at_ms = rows
        .iter()
        .filter_map(|row| row.timestamp.as_deref())
        .map(ms)
        .min();
    // The retention setting first: a saved title is stored content.
    store
        .set_retention_mode(if keep {
            RetentionMode::FullContent
        } else {
            RetentionMode::MetadataOnly
        })
        .unwrap();
    store.upsert_session(&meta, keep).unwrap();
    store.upsert_records(id, rows, keep).unwrap();
    // A closed synthetic catalog, as a fixture database is: no history a
    // child detector reads exists, so each display check is finished.
    let key = "u1:fixture";
    if let Some(attempt) = store.observe_own_check(id, Some(key)).unwrap() {
        store
            .settle_child_checks(&[(id, attempt.required, key)])
            .unwrap();
    }
}

const DECLINED: &str =
    "The user doesn't want to proceed with this tool use. The tool use was rejected.";

/// The synthetic sessions, each a case a screen must keep apart:
///
/// * `s-warned`: a correction on Sep 4 (outside the lanes' 48 hours) and, on
///   Sep 7, a repeated failure, a declined call, a correction and a repeated
///   prompt; every check read everything. Its six identical reads on Sep 7
///   are no warning (there is no repeated-calls rule).
/// * `s-unchecked`: a Codex session indexed without its words, whose
///   results never say whether they failed: no warning, and two checks could
///   not read everything.
/// * `s-clean`: no warning, every check complete.
/// * `s-older`: a correction on Sep 3, inside the 7-day range only.
fn seed(path: &Path) {
    let mut store = Store::open(path).unwrap();
    let at =
        |day: u32, hour: u32, minute: u32| format!("2026-09-{day:02}T{hour:02}:{minute:02}:00Z");
    let bash = json!({"command": "pnpm test"});
    let read = json!({"file_path": "/work/atlas/src/build.rs"});
    session(
        &mut store,
        "s-warned",
        "claude",
        "Fix the flaky build",
        &[
            says("w-h0", &at(4, 10, 0), "rename the build step to compile"),
            reply("w-a0", &at(4, 10, 2)),
            says("w-h1", &at(4, 10, 5), "no, the other step"),
            reply("w-a1", &at(4, 10, 8)),
            says("w-h2", &at(7, 14, 0), "fix the build please"),
            calls("w-c1", &at(7, 14, 1), &[("t1", "Bash", bash.clone())]),
            result("w-r1", &at(7, 14, 2), "t1", Some(true), "1 test failed"),
            calls("w-c2", &at(7, 14, 3), &[("t2", "Bash", bash.clone())]),
            result("w-r2", &at(7, 14, 4), "t2", Some(true), "1 test failed"),
            calls(
                "w-c3",
                &at(7, 14, 5),
                &[("t3", "Edit", json!({"file_path": "/work/atlas/ci.yml"}))],
            ),
            result("w-r3", &at(7, 14, 6), "t3", Some(true), DECLINED),
            says(
                "w-i1",
                &at(7, 14, 7),
                "[Request interrupted by user for tool use]",
            ),
            says("w-h3", &at(7, 15, 0), "still not working"),
            reply("w-a2", &at(7, 15, 30)),
            says("w-h4", &at(7, 16, 0), "fix the build please"),
            calls(
                "w-c4",
                &at(7, 16, 1),
                &[
                    ("t4", "Read", read.clone()),
                    ("t5", "Read", read.clone()),
                    ("t6", "Read", read.clone()),
                    ("t7", "Read", read.clone()),
                    ("t8", "Read", read.clone()),
                    ("t9", "Read", read),
                ],
            ),
            reply("w-a3", &at(7, 16, 5)),
        ],
        true,
    );
    let exec = json!({"cmd": "cargo test"});
    session(
        &mut store,
        "s-unchecked",
        "codex",
        "Port the parser",
        &[
            says("u-h1", &at(7, 9, 0), "port the parser to the new API"),
            calls(
                "u-c1",
                &at(7, 9, 1),
                &[("c1", "exec_command", exec.clone())],
            ),
            result("u-r1", &at(7, 9, 2), "c1", None, "Exit code: 1"),
            calls("u-c2", &at(7, 9, 3), &[("c2", "exec_command", exec)]),
            result("u-r2", &at(7, 9, 4), "c2", None, "Exit code: 1"),
            reply("u-a1", &at(7, 9, 40)),
        ],
        false,
    );
    session(
        &mut store,
        "s-clean",
        "claude",
        "Write the release notes",
        &[
            says("c-h1", &at(7, 11, 0), "write the release notes for 0.2"),
            calls(
                "c-c1",
                &at(7, 11, 1),
                &[("r1", "Read", json!({"file_path": "/work/atlas/CHANGES.md"}))],
            ),
            result("c-r1", &at(7, 11, 2), "r1", Some(false), "ok"),
            reply("c-a1", &at(7, 11, 25)),
        ],
        true,
    );
    session(
        &mut store,
        "s-older",
        "claude",
        "Rename the config module",
        &[
            says("o-h1", &at(3, 9, 0), "please rename the config module"),
            reply("o-a1", &at(3, 9, 20)),
            says("o-h2", &at(3, 9, 30), "that is wrong, use settings"),
            reply("o-a2", &at(3, 9, 50)),
        ],
        true,
    );
}

struct WindowReport {
    window_days: u32,
    window_end_ms: i64,
    timezone: String,
    warnings: DashboardWarnings,
}

fn window_reports(
    path: &Path,
    now_ms: i64,
) -> Result<Vec<WindowReport>, xtrace_desktop::state::StateError> {
    let home = tempfile::tempdir().unwrap();
    let state = xtrace_desktop::state::AppState::build(
        xtrace_desktop::state::StartupOptions {
            data_dir: Some(path.parent().unwrap().to_owned()),
            native_home: Some(home.path().to_owned()),
            ..Default::default()
        },
        || panic!("explicit synthetic database"),
        || panic!("explicit synthetic home"),
    )?;
    let reports = xtrace_desktop::dashboard::WINDOW_PRESETS
        .into_iter()
        .map(|days| {
            Ok(WindowReport {
                window_days: days,
                window_end_ms: now_ms,
                timezone: "UTC".into(),
                warnings: state.window_warnings(days, now_ms, "UTC")?,
            })
        })
        .collect();
    state.shutdown();
    reports
}

fn answers(list: &SessionWarningsList) -> std::collections::BTreeMap<&str, &SessionWarningsAnswer> {
    list.sessions
        .iter()
        .map(|entry| (entry.session_id.as_str(), &entry.answer))
        .collect()
}

fn warm_saved(path: &Path) {
    let metrics = xt_metrics::MetricsDb::open(path).unwrap();
    if let Some(candidate) = metrics.prepare_session_warning_refresh(&[]).unwrap() {
        assert!(
            Store::open(path)
                .unwrap()
                .publish_session_warnings(&candidate.rows)
                .unwrap()
                .rejected
                == 0
        );
    }
}

fn database() -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("xtrace.db");
    seed(&path);
    warm_saved(&path);
    (root, path)
}

#[test]
fn every_screen_reads_one_worded_answer_per_session() {
    let (_root, path) = database();
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let list = session_warnings(
        &metrics,
        &["s-warned", "s-unchecked", "s-clean", "nobody"],
        &jiff::tz::TimeZone::UTC,
        "UTC".into(),
    )
    .unwrap();
    assert_eq!(list.timezone, "UTC");
    let found = answers(&list);
    assert_eq!(found["nobody"], &SessionWarningsAnswer::Missing);

    let SessionWarningsAnswer::Read {
        warnings,
        count_label,
        summary,
        checks_summary,
        dismissed,
        checks,
        incomplete_checks,
    } = found["s-warned"]
    else {
        panic!("read");
    };
    assert!(dismissed.is_empty(), "nothing was dismissed");
    let shown: Vec<_> = warnings
        .iter()
        .map(|w| (w.kind, w.label.as_str(), w.reason.as_str()))
        .collect();
    assert_eq!(
        shown,
        vec![
            (
                SessionWarningKind::Correction,
                "Correction",
                "you wrote \"no\" after a reply (10:05)"
            ),
            (
                SessionWarningKind::RepeatedFailure,
                "Repeated failure",
                "the same Bash call failed twice (14:02, 14:04)"
            ),
            (
                SessionWarningKind::Interruption,
                "Interruption",
                "you declined a call to Edit (14:06)"
            ),
            (
                SessionWarningKind::Correction,
                "Correction",
                "you wrote \"still not\" after a reply (15:00)"
            ),
            (
                SessionWarningKind::RepeatedPrompt,
                "Repeated prompt",
                "same prompt sent twice (14:00, 16:00)"
            ),
        ]
    );
    // Nothing a screen shows says "circling": that is M-20's own word.
    let json = serde_json::to_string(&list).unwrap();
    assert!(!json.to_lowercase().contains("circl"), "{json}");
    // Each warning has its own identity, from saved IDs only.
    let ids: Vec<_> = warnings.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "correction:w-h1",
            "repeated_failure:w-r1#0",
            "interruption:w-r3#0",
            "correction:w-h3",
            "repeated_prompt:w-h2",
        ]
    );
    assert_eq!(count_label.as_deref(), Some("5 warnings"));
    assert_eq!(*incomplete_checks, 0);
    assert_eq!(checks_summary, "Checks · all 5 complete");
    assert_eq!(summary, "5 warnings to review.");
    assert_eq!(checks.len(), 5);
    assert!(!json.contains("repeated_calls") && !json.contains("Repeated call"));
    assert!(checks.iter().all(|check| check.complete));

    // No warning, but not everything could be read: never "no warnings".
    let SessionWarningsAnswer::Read {
        warnings,
        count_label,
        summary,
        checks_summary,
        checks,
        incomplete_checks,
        ..
    } = found["s-unchecked"]
    else {
        panic!("read");
    };
    assert!(warnings.is_empty());
    assert_eq!(count_label, &None);
    assert_eq!(*incomplete_checks, 3);
    assert_eq!(checks_summary, "Checks · 3 of 5 incomplete");
    assert_eq!(
        summary,
        "No warnings found, but 3 of 5 checks could not read everything, so something may still have gone wrong."
    );
    let texts: Vec<_> = checks.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(
        texts,
        vec![
            // Its one message opens the session: no reply came before it.
            "Checked: corrections",
            "Not fully checked: interruptions — 2 tool results do not say how they ended",
            "Not fully checked: repeated prompts — the words of 1 of your messages were not kept",
            "Not fully checked: repeated failures — 2 tool results do not say how they ended",
            "Checked: admitted mistakes",
        ]
    );

    let SessionWarningsAnswer::Read {
        count_label,
        summary,
        ..
    } = found["s-clean"]
    else {
        panic!("read");
    };
    assert_eq!(count_label, &None);
    assert_eq!(
        summary,
        "No warnings. Every check read everything it needed."
    );
}

#[test]
fn a_session_shows_the_same_warnings_whichever_range_asks() {
    let (_root, path) = database();
    let now = ms(NOW);
    let effort = window_reports(&path, now).unwrap();
    let in_range = |days: u32| -> Vec<(String, Vec<(String, String)>)> {
        effort
            .iter()
            .find(|entry| entry.window_days == days)
            .unwrap()
            .warnings
            .days
            .iter()
            .map(|day| {
                (
                    day.date.clone(),
                    day.warnings
                        .iter()
                        .map(|w| (w.session_id.clone(), w.warning.id.clone()))
                        .collect(),
                )
            })
            .collect()
    };
    let pair = |session: &str, id: &str| (session.to_owned(), id.to_owned());
    // Seven days hold Sep 1–7: every warning of both warned sessions, on the
    // day it falls on, earliest first; days without one are left out.
    assert_eq!(
        in_range(7),
        vec![
            (
                "2026-09-03".to_owned(),
                vec![pair("s-older", "correction:o-h2")]
            ),
            (
                "2026-09-04".to_owned(),
                vec![pair("s-warned", "correction:w-h1")]
            ),
            (
                "2026-09-07".to_owned(),
                vec![
                    pair("s-warned", "repeated_failure:w-r1#0"),
                    pair("s-warned", "interruption:w-r3#0"),
                    pair("s-warned", "correction:w-h3"),
                    pair("s-warned", "repeated_prompt:w-h2"),
                ]
            ),
        ]
    );
    // The same warnings, with the same words, as the session's own answer.
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let own = session_warnings(
        &metrics,
        &["s-warned"],
        &jiff::tz::TimeZone::UTC,
        "UTC".into(),
    )
    .unwrap();
    let SessionWarningsAnswer::Read { warnings, .. } = &own.sessions[0].answer else {
        panic!("read");
    };
    let week = effort.iter().find(|entry| entry.window_days == 7).unwrap();
    let mut ranged: Vec<_> = week
        .warnings
        .days
        .iter()
        .flat_map(|day| &day.warnings)
        .filter(|entry| entry.session_id == "s-warned")
        .collect();
    // What the UI names the session by, as the lanes do: never a name made
    // here.
    assert!(ranged.iter().all(|entry| {
        entry.title.as_deref() == Some("Fix the flaky build")
            && entry.host == "claude"
            && !entry.automated_review
            && entry.last_ms == ms("2026-09-07T16:05:00Z")
    }));
    ranged.sort_by_key(|entry| warnings.iter().position(|w| *w == entry.warning));
    let ranged: Vec<_> = ranged.into_iter().map(|entry| &entry.warning).collect();
    assert_eq!(ranged, warnings.iter().collect::<Vec<_>>());
    // Each day's count and listing, and each warning's words without its
    // session, from the answer itself.
    let sep7 = week
        .warnings
        .days
        .iter()
        .find(|day| day.date == "2026-09-07")
        .unwrap();
    assert_eq!(sep7.listed, 4);
    assert_eq!(sep7.count_label, "4 warnings");
    assert_eq!(sep7.more_label, None);
    assert_eq!(
        sep7.warnings[0].text,
        "Repeated failure: the same Bash call failed twice (14:02, 14:04)"
    );
    let sep3 = &week.warnings.days[0];
    assert_eq!(sep3.count_label, "1 warning");
    assert_eq!(
        sep3.warnings[0].text,
        "Correction: you wrote \"wrong\" after a reply (09:30)"
    );
    assert_eq!(
        sep3.warnings[0].title.as_deref(),
        Some("Rename the config module")
    );
    // A dot's own name says its day, label and reason.
    assert_eq!(
        sep3.warnings[0].warning.name,
        "Warning. Sep 3, Correction: you wrote \"wrong\" after a reply (09:30)"
    );
    for entry in &effort {
        assert_eq!(entry.timezone, "UTC");
        assert_eq!(entry.window_end_ms, now);
    }
}

/// A report's days are local days: a warning at local midnight is the next
/// day's first moment, and a day when the clocks change is 23 or 25 hours
/// long, with every warning inside it on that day.
#[test]
fn warnings_fall_on_the_local_day_they_happened_across_midnight_and_clock_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("xtrace.db");
    {
        let mut store = Store::open(&path).unwrap();
        // A correction at 23:30 local, and one at exactly 00:00 local the
        // next day, on the day each clock change happens.
        let corrected = |store: &mut Store, id: &str, asked: &str, reply_at: &str, no: &str| {
            session(
                store,
                id,
                "claude",
                id,
                &[
                    says(&format!("{id}-1"), asked, "please rename the module"),
                    reply(&format!("{id}-2"), reply_at),
                    says(&format!("{id}-3"), no, "no, the other module"),
                ],
                true,
            );
        };
        // Sunday 8 March 2026: Los Angeles moves from -08:00 to -07:00.
        corrected(
            &mut store,
            "spring-late",
            "2026-03-08T23:00:00-07:00",
            "2026-03-08T23:10:00-07:00",
            "2026-03-08T23:30:00-07:00",
        );
        // Sunday 1 November 2026: back from -07:00 to -08:00.
        corrected(
            &mut store,
            "autumn-late",
            "2026-11-01T23:00:00-08:00",
            "2026-11-01T23:10:00-08:00",
            "2026-11-01T23:30:00-08:00",
        );
        corrected(
            &mut store,
            "autumn-midnight",
            "2026-11-01T23:40:00-08:00",
            "2026-11-01T23:50:00-08:00",
            "2026-11-02T00:00:00-08:00",
        );
    }
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let zone = jiff::tz::TimeZone::get("America/Los_Angeles").unwrap();
    let read = |from: &str, to: &str| {
        let window = xt_metrics::Window::new(ms(from), ms(to)).unwrap();
        window_warnings(&metrics, window, &zone, "America/Los_Angeles".into()).unwrap()
    };
    let hours =
        |day: &xtrace_desktop::dto::DashboardWarningDay| (day.end_ms - day.start_ms) / 3_600_000;
    let days_of =
        |answer: &xtrace_desktop::dto::DashboardWarnings| -> Vec<(String, i64, Vec<String>)> {
            answer
                .days
                .iter()
                .map(|day| {
                    (
                        day.date.clone(),
                        hours(day),
                        day.warnings.iter().map(|w| w.session_id.clone()).collect(),
                    )
                })
                .collect()
        };

    let spring = read("2026-03-07T00:00:00-08:00", "2026-03-10T00:00:00-07:00");
    assert_eq!(
        days_of(&spring),
        vec![("2026-03-08".to_owned(), 23, vec!["spring-late".to_owned()])]
    );

    let autumn = read("2026-10-31T00:00:00-07:00", "2026-11-03T00:00:00-08:00");
    assert_eq!(
        days_of(&autumn),
        vec![
            ("2026-11-01".to_owned(), 25, vec!["autumn-late".to_owned()]),
            (
                "2026-11-02".to_owned(),
                24,
                vec!["autumn-midnight".to_owned()]
            ),
        ]
    );
    // The midnight warning is the next day's first moment: position 0.
    let next = &autumn.days[1];
    assert_eq!(next.warnings[0].warning.at_ms, next.start_ms);
    assert_eq!(next.start_ms, ms("2026-11-02T00:00:00-08:00"));
    assert_eq!(next.warnings[0].warning.day, "Nov 2");
    // And the late one sits in the 25-hour day, before its end.
    let late = &autumn.days[0];
    assert!(late.warnings[0].warning.at_ms < late.end_ms);
    assert_eq!(late.end_ms, next.start_ms);
}

#[test]
fn a_request_names_at_most_one_page_of_canonical_identifiers() {
    let many: Vec<String> = (0..=200).map(|index| format!("s{index}")).collect();
    assert!(validate_ids(&many).is_err());
    // Duplicates collapse before the bound is applied.
    let repeated: Vec<String> = (0..300).map(|_| "s".to_owned()).collect();
    assert_eq!(validate_ids(&repeated).unwrap(), vec!["s"]);
    for bad in ["", " s", "s ", &"x".repeat(257)] {
        assert!(validate_ids(&[bad.to_owned()]).is_err(), "{bad:?}");
    }
}

#[test]
fn a_session_with_no_timed_record_is_never_said_to_have_no_warnings() {
    let SessionWarningsAnswer::Read {
        warnings,
        count_label,
        summary,
        checks,
        incomplete_checks,
        ..
    } = xtrace_desktop::session_warnings_dto::answer(
        xt_metrics::WholeSessionWarnings::Untimed { untimed_records: 3 },
        &Default::default(),
        &jiff::tz::TimeZone::UTC,
    )
    else {
        panic!("read");
    };
    assert!(warnings.is_empty());
    assert_eq!(count_label, None);
    assert_eq!(incomplete_checks, 5);
    assert_eq!(
        summary,
        "Not checked: none of this session's 3 saved records says when it happened, so no warning can be found and none is shown."
    );
    assert!(checks.iter().all(|check| !check.complete));
    assert_eq!(
        checks[3].text,
        "Not checked: repeated failures — none of this session's 3 saved records says when it happened"
    );
}

/// The ids and the dismissed ids of a read answer, and its labels.
fn shown(answer: &SessionWarningsAnswer) -> (Vec<&str>, Vec<&str>, Option<&str>, &str) {
    let SessionWarningsAnswer::Read {
        warnings,
        dismissed,
        count_label,
        summary,
        ..
    } = answer
    else {
        panic!("read");
    };
    (
        warnings.iter().map(|w| w.id.as_str()).collect(),
        dismissed.iter().map(|w| w.id.as_str()).collect(),
        count_label.as_deref(),
        summary.as_str(),
    )
}

#[test]
fn a_dismissed_warning_is_listed_apart_and_counted_nowhere_else() {
    let (_root, path) = database();
    {
        let mut store = Store::open(&path).unwrap();
        assert!(
            store
                .dismiss_warning("s-warned", "correction:w-h3", None, 1)
                .unwrap()
        );
        assert!(
            store
                .dismiss_warning("s-older", "correction:o-h2", None, 1)
                .unwrap()
        );
        // A dismissal whose warning is not found hides nothing and is not listed.
        assert!(
            store
                .dismiss_warning("s-clean", "correction:gone", None, 1)
                .unwrap()
        );
    }
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let zone = jiff::tz::TimeZone::UTC;
    let list = session_warnings(
        &metrics,
        &["s-warned", "s-older", "s-clean", "s-unchecked"],
        &zone,
        "UTC".into(),
    )
    .unwrap();
    let found = answers(&list);
    assert_eq!(
        shown(found["s-warned"]),
        (
            vec![
                "correction:w-h1",
                "repeated_failure:w-r1#0",
                "interruption:w-r3#0",
                "repeated_prompt:w-h2",
            ],
            vec!["correction:w-h3"],
            Some("4 warnings"),
            "4 warnings to review."
        )
    );
    assert_eq!(
        shown(found["s-older"]),
        (
            vec![],
            vec!["correction:o-h2"],
            None,
            "No warnings left: you dismissed 1. Every check read everything it needed."
        )
    );
    assert_eq!(
        shown(found["s-clean"]),
        (
            vec![],
            vec![],
            None,
            "No warnings. Every check read everything it needed."
        )
    );
    // A dismissed warning keeps its words on the page that lists it.
    let SessionWarningsAnswer::Read { dismissed, .. } = found["s-warned"] else {
        panic!("read");
    };
    assert_eq!(
        dismissed[0].reason,
        "you wrote \"still not\" after a reply (15:00)"
    );

    // The Effort card's days leave it out of their dots, counts and names.
    let week = || {
        window_reports(&path, ms(NOW))
            .unwrap()
            .into_iter()
            .find(|entry| entry.window_days == 7)
            .unwrap()
            .warnings
    };
    let days = week();
    let dates: Vec<_> = days.days.iter().map(|d| d.date.as_str()).collect();
    assert_eq!(
        dates,
        ["2026-09-04", "2026-09-07"],
        "Sep 3's only warning is dismissed"
    );
    let seventh = &days.days[1];
    assert_eq!(seventh.count_label, "3 warnings");
    assert_eq!(seventh.listed, 3);
    assert!(
        seventh
            .warnings
            .iter()
            .all(|entry| entry.warning.id != "correction:w-h3")
    );
    drop(metrics);

    // Undoing deletes the row: the warning is back everywhere.
    let mut store = Store::open(&path).unwrap();
    assert!(
        store
            .restore_warning("s-warned", "correction:w-h3")
            .unwrap()
    );
    drop(store);
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let list = session_warnings(&metrics, &["s-warned"], &zone, "UTC".into()).unwrap();
    let (ids, gone, count, _) = shown(&list.sessions[0].answer);
    assert_eq!((ids.len(), gone.len(), count), (5, 0, Some("5 warnings")));
    let days = week();
    assert_eq!(days.days.last().unwrap().count_label, "4 warnings");
}

/// A dismissal keeps the warning's size: a dismissed repeated prompt or
/// repeated failure stays dismissed while it is no bigger, and shows again
/// everywhere, saying so on its page, once it has grown past it.
#[test]
fn a_dismissed_warning_that_grows_shows_again() {
    let (_root, path) = database();
    let zone = jiff::tz::TimeZone::UTC;
    let ids = ["repeated_failure:w-r1#0", "repeated_prompt:w-h2"];
    let read = |path: &Path| {
        warm_saved(path);
        let metrics = xt_metrics::MetricsDb::open(path).unwrap();
        let list = session_warnings(&metrics, &["s-warned"], &zone, "UTC".into()).unwrap();
        let SessionWarningsAnswer::Read {
            warnings,
            dismissed,
            summary,
            ..
        } = list.sessions[0].answer.clone()
        else {
            panic!("read");
        };
        let window =
            xt_metrics::Window::new(ms("2026-09-01T00:00:00Z"), ms("2026-09-08T00:00:00Z"))
                .unwrap();
        let effort: Vec<String> = window_warnings(&metrics, window, &zone, "UTC".into())
            .unwrap()
            .days
            .into_iter()
            .flat_map(|day| day.warnings)
            .filter(|entry| entry.session_id == "s-warned")
            .map(|entry| entry.warning.id)
            .collect();
        (warnings, dismissed, summary, effort)
    };
    {
        let mut store = Store::open(&path).unwrap();
        // Each is dismissed at its size now: sent twice, failed twice.
        for id in ids {
            assert!(store.dismiss_warning("s-warned", id, Some(2), 1).unwrap());
        }
    }
    // As big as when dismissed: still dismissed, nowhere else.
    let (warnings, dismissed, _, effort) = read(&path);
    let mut gone: Vec<_> = dismissed.iter().map(|w| w.id.as_str()).collect();
    gone.sort_unstable();
    assert_eq!(gone, ids);
    assert!(warnings.iter().all(|w| !ids.contains(&w.id.as_str())));
    assert!(effort.iter().all(|id| !ids.contains(&id.as_str())));
    assert!(dismissed.iter().all(|w| w.returned.is_none()));
    // A newer dismissal at 3 remains effective if a re-read shrinks to 2.
    {
        let mut store = Store::open(&path).unwrap();
        assert!(
            store
                .dismiss_warning("s-warned", ids[0], Some(3), 2)
                .unwrap()
        );
    }
    let (warnings, dismissed, _, effort) = read(&path);
    assert!(dismissed.iter().any(|warning| warning.id == ids[0]));
    assert!(!warnings.iter().any(|warning| warning.id == ids[0]));
    assert!(!effort.iter().any(|id| id == ids[0]));
    {
        let mut store = Store::open(&path).unwrap();
        assert!(store.restore_warning("s-warned", ids[0]).unwrap());
        assert!(
            store
                .dismiss_warning("s-warned", ids[0], Some(2), 1)
                .unwrap()
        );
    }
    // Both grow: the call fails a third time and the prompt is sent again.
    let bash = json!({"command": "pnpm test"});
    let mut store = Store::open(&path).unwrap();
    store
        .upsert_records(
            "s-warned",
            &[
                calls(
                    "w-c5",
                    "2026-09-07T16:10:00Z",
                    &[("t10", "Bash", bash.clone())],
                ),
                result(
                    "w-r5",
                    "2026-09-07T16:11:00Z",
                    "t10",
                    Some(true),
                    "1 test failed",
                ),
                says("w-h5", "2026-09-07T17:00:00Z", "fix the build please"),
                reply("w-a5", "2026-09-07T17:05:00Z"),
            ],
            true,
        )
        .unwrap();
    drop(store);
    let (warnings, dismissed, summary, effort) = read(&path);
    assert!(dismissed.is_empty(), "{dismissed:?}");
    let back: Vec<_> = warnings
        .iter()
        .filter(|w| ids.contains(&w.id.as_str()))
        .map(|w| (w.id.as_str(), w.returned.as_deref()))
        .collect();
    assert_eq!(
        back,
        [
            (
                "repeated_failure:w-r1#0",
                Some(
                    "Shown again: you dismissed it at 2 failures; the call has now failed 3 times."
                )
            ),
            (
                "repeated_prompt:w-h2",
                Some(
                    "Shown again: you dismissed it when the prompt had been sent 2 times; it has now been sent 3 times."
                )
            ),
        ]
    );
    // Counted again, and on its Effort day.
    assert!(summary.starts_with("5 warnings"), "{summary}");
    for id in ids {
        assert!(effort.iter().any(|shown| shown == id), "{id} {effort:?}");
    }
    // Dismissed again at its new size; undone, it shows with no note.
    let mut store = Store::open(&path).unwrap();
    assert!(
        store
            .dismiss_warning("s-warned", ids[0], Some(3), 2)
            .unwrap()
    );
    let (_, dismissed, _, _) = read(&path);
    assert_eq!(
        dismissed.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        [ids[0]]
    );
    assert!(store.restore_warning("s-warned", ids[0]).unwrap());
    let (warnings, dismissed, _, _) = read(&path);
    assert!(dismissed.is_empty());
    let failure = warnings.iter().find(|w| w.id == ids[0]).unwrap();
    assert_eq!(failure.returned, None);
}

/// Exact warning targets used by the transcript UI tests, decided in Rust.
#[test]
fn warning_targets_preserve_the_exact_record_and_result_block() {
    let (_root, path) = database();
    let mut store = Store::open(&path).unwrap();
    session(
        &mut store,
        "s-two-results",
        "claude",
        "Two failed calls",
        &[
            calls(
                "two-calls",
                "2026-09-07T12:00:00Z",
                &[
                    ("first", "Bash", json!({"command": "pnpm test"})),
                    ("second", "Bash", json!({"command": "pnpm test"})),
                ],
            ),
            record(
                json!({"uuid":"two-results", "type":"user", "timestamp":"2026-09-07T12:01:00Z",
                "message":{"role":"user", "content":[
                    {"type":"tool_result", "tool_use_id":"first", "is_error":true, "content":"first failure"},
                    {"type":"tool_result", "tool_use_id":"second", "is_error":true, "content":"second failure"}
                ]}}),
            ),
        ],
        true,
    );
    drop(store);
    let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
    let list = session_warnings(
        &metrics,
        &["s-warned", "s-two-results"],
        &jiff::tz::TimeZone::UTC,
        "UTC".into(),
    )
    .unwrap();
    let warnings: Vec<_> = list
        .sessions
        .into_iter()
        .flat_map(|session| match session.answer {
            SessionWarningsAnswer::Read { warnings, .. } => warnings,
            _ => panic!("read"),
        })
        .filter(|warning| {
            warning.id == "correction:w-h1" || warning.id == "repeated_failure:two-results#0"
        })
        .collect();
    assert_eq!(warnings.len(), 2);
    let failed = warnings
        .iter()
        .find(|warning| warning.kind == SessionWarningKind::RepeatedFailure)
        .unwrap();
    assert_eq!(failed.points_at.record_uuid, "two-results");
    assert_eq!(failed.points_at.block, Some(1));
}

fn live(root: &Path) -> xtrace_desktop::state::AppState {
    warm_saved(&root.join("data/xtrace.db"));
    std::fs::create_dir_all(root.join("home")).unwrap();
    xtrace_desktop::state::AppState::build(
        xtrace_desktop::state::StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

#[test]
fn the_commands_dismiss_only_a_warning_the_session_has_and_undo_it() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    seed(&root.path().join("data/xtrace.db"));
    let state = live(root.path());
    let refused = |session: &str, warning: &str| {
        state
            .dismiss_session_warning(session, warning)
            .unwrap_err()
            .to_string()
    };
    // Shapes no warning could have are refused before storage is touched.
    let long = format!("correction:{}", "r".repeat(600));
    for (session, warning) in [
        ("", "correction:w-h1"),
        (" s-warned", "correction:w-h1"),
        ("s-warned", ""),
        ("s-warned", "correction:"),
        ("s-warned", "circling:w-h1"),
        // The removed kind: no warning has it any more.
        ("s-warned", "repeated_calls:w-h4"),
        ("s-warned", "w-h1"),
        ("s-warned", long.as_str()),
    ] {
        assert_eq!(
            refused(session, warning),
            "a dismissal names one session and one warning of it",
            "{session:?} {warning:?}"
        );
        assert!(state.restore_session_warning(session, warning).is_err());
    }
    // A well-formed identity the session does not have now is refused too.
    for (session, warning) in [
        ("s-warned", "correction:w-h9"),
        ("nobody", "correction:w-h1"),
    ] {
        assert_eq!(
            refused(session, warning),
            "this session has no such warning now"
        );
    }
    let ids = |state: &xtrace_desktop::state::AppState| {
        let list = state
            .session_warnings(&["s-warned".to_owned()], "UTC")
            .unwrap();
        let (active, gone, _, _) = shown(&list.sessions[0].answer);
        (
            active.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            gone.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    };
    assert_eq!(ids(&state).0.len(), 5);
    // The one lookup a dismissal checks with: the same warning in any zone,
    // only its reason's clock times differ; none for one it does not have.
    let found = |zone: &str| {
        state
            .session_warning("s-warned", "interruption:w-r3#0", zone)
            .unwrap()
            .map(|(warning, _)| warning)
    };
    let (utc, tokyo) = (found("UTC").unwrap(), found("Asia/Tokyo").unwrap());
    assert_eq!(
        (utc.id(), utc.at_ms, utc.record_uuids.clone()),
        (tokyo.id(), tokyo.at_ms, tokyo.record_uuids.clone())
    );
    assert!(
        state
            .session_warning("s-warned", "correction:w-h9", "UTC")
            .unwrap()
            .is_none()
    );
    let done = state
        .dismiss_session_warning("s-warned", "interruption:w-r3#0")
        .unwrap();
    assert!(done.dismissed);
    assert_eq!(done.warning_id, "interruption:w-r3#0");
    // Dismissing it again changes nothing.
    assert!(
        state
            .dismiss_session_warning("s-warned", "interruption:w-r3#0")
            .unwrap()
            .dismissed
    );
    let (active, gone) = ids(&state);
    assert_eq!(active.len(), 4);
    assert_eq!(gone, ["interruption:w-r3#0"]);
    let undone = state
        .restore_session_warning("s-warned", "interruption:w-r3#0")
        .unwrap();
    assert!(!undone.dismissed);
    assert_eq!(ids(&state), (active_all(), vec![]));
    // Undoing what is not dismissed changes nothing.
    assert!(
        !state
            .restore_session_warning("s-warned", "correction:w-h1")
            .unwrap()
            .dismissed
    );
    // The command stores a growing warning's size now (sent twice), and
    // none for a one-event warning.
    state
        .dismiss_session_warning("s-warned", "repeated_prompt:w-h2")
        .unwrap();
    state
        .dismiss_session_warning("s-warned", "correction:w-h1")
        .unwrap();
    let sizes: Vec<(String, Option<i64>)> =
        rusqlite::Connection::open(root.path().join("data/xtrace.db"))
            .unwrap()
            .prepare("SELECT warning_id,size FROM dismissed_warnings ORDER BY warning_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
    assert_eq!(
        sizes,
        [
            ("correction:w-h1".to_owned(), None),
            ("repeated_prompt:w-h2".to_owned(), Some(2)),
        ]
    );
}

fn active_all() -> Vec<String> {
    [
        "correction:w-h1",
        "repeated_failure:w-r1#0",
        "interruption:w-r3#0",
        "correction:w-h3",
        "repeated_prompt:w-h2",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn saved_missing_and_invalid_payload_answers_equal_the_direct_answer() {
    let (_root, path) = database();
    let ids = ["s-warned", "s-unchecked", "s-clean", "s-older", "absent"];
    let sql = rusqlite::Connection::open(&path).unwrap();
    for zone_name in ["UTC", "America/Los_Angeles", "Asia/Tokyo"] {
        let zone = jiff::tz::TimeZone::get(zone_name).unwrap();
        let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
        let direct = metrics.whole_session_warnings(&ids, &zone).unwrap();
        let expected: Vec<_> = direct
            .into_iter()
            .map(|(id, found)| {
                (
                    id,
                    xtrace_desktop::session_warnings_dto::answer(found, &Default::default(), &zone),
                )
            })
            .collect();
        for corrupt in [false, true] {
            if corrupt {
                sql.execute("UPDATE saved_session_warnings SET payload_json='{}'", [])
                    .unwrap();
            } else {
                warm_saved(&path);
            }
            let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
            let list = session_warnings(&metrics, &ids, &zone, zone_name.into()).unwrap();
            let actual: Vec<_> = list
                .sessions
                .into_iter()
                .map(|entry| (entry.session_id, entry.answer))
                .collect();
            assert_eq!(actual, expected);
            assert_eq!(metrics.session_warning_refresh_needed(), corrupt);
        }
        sql.execute("DELETE FROM saved_session_warnings", [])
            .unwrap();
        let metrics = xt_metrics::MetricsDb::open(&path).unwrap();
        let fresh = session_warnings(&metrics, &ids, &zone, zone_name.into()).unwrap();
        assert_eq!(
            fresh
                .sessions
                .into_iter()
                .map(|entry| (entry.session_id, entry.answer))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(metrics.session_warning_refresh_needed());
    }
}

#[test]
fn excluded_sessions_disappear_from_both_answers_and_cannot_be_dismissed() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/xtrace.db");
    seed(&path);
    warm_saved(&path);
    let sql = rusqlite::Connection::open(&path).unwrap();
    // Classification belongs to the existing index; this fixture makes one
    // formerly visible session non-user without opening a host history.
    sql.execute(
        "UPDATE sessions SET kind='judge' WHERE session_id='s-warned'",
        [],
    )
    .unwrap();
    let state = live(root.path());
    let list = state.session_warnings(&["s-warned".into()], "UTC").unwrap();
    assert_eq!(list.sessions[0].answer, SessionWarningsAnswer::Missing);
    let window = state.window_warnings(7, ms(NOW), "UTC").unwrap();
    assert!(
        window
            .days
            .iter()
            .flat_map(|day| &day.warnings)
            .all(|entry| entry.session_id != "s-warned")
    );
    assert!(matches!(
        state.dismiss_session_warning("s-warned", "correction:w-h1"),
        Err(xtrace_desktop::state::StateError::UnknownWarning)
    ));
    let count: i64 = sql
        .query_row("SELECT count(*) FROM dismissed_warnings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    state.shutdown();
}

/// Dismissals stored before the repeated-calls rule was removed stay in the
/// table (no migration touches it): they name no warning any answer has, so
/// they are never shown, counted or offered for undo, and more identical
/// calls never bring a warning back.
#[test]
fn an_old_repeated_calls_dismissal_is_harmless_and_more_repeats_add_no_warning() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/xtrace.db");
    seed(&path);
    let state = live(root.path());
    let read = || state.session_warnings(&["s-warned".into()], "UTC").unwrap();
    let week = || state.window_warnings(7, ms(NOW), "UTC").unwrap();
    let before = (read(), week());
    {
        // As the app stored it when the warning existed, at 5 repeats.
        let mut store = Store::open(&path).unwrap();
        assert!(
            store
                .dismiss_warning("s-warned", "repeated_calls:w-h4", Some(5), 1)
                .unwrap()
        );
    }
    let stored: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM dismissed_warnings WHERE warning_id='repeated_calls:w-h4'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 1);
    // Every answer is exactly what it was without the row.
    assert_eq!((read(), week()), before);
    let (active, dismissed, count, summary) = shown(&before.0.sessions[0].answer);
    assert_eq!(active, active_all());
    assert!(dismissed.is_empty());
    assert_eq!(count, Some("5 warnings"));
    assert_eq!(summary, "5 warnings to review.");
    // Its undo, should anything send one, is refused like any unknown kind.
    assert!(
        state
            .restore_session_warning("s-warned", "repeated_calls:w-h4")
            .is_err()
    );
    // A seventh identical read: still no warning about repeats.
    let mut store = Store::open(&path).unwrap();
    store
        .upsert_records(
            "s-warned",
            &[calls(
                "w-extra",
                "2026-09-07T16:06:00Z",
                &[(
                    "extra",
                    "Read",
                    json!({"file_path":"/work/atlas/src/build.rs"}),
                )],
            )],
            true,
        )
        .unwrap();
    drop(store);
    let grown = read();
    let (active, dismissed, _, _) = shown(&grown.sessions[0].answer);
    assert_eq!(active, active_all());
    assert!(dismissed.is_empty());
    let json = serde_json::to_string(&(grown, week())).unwrap();
    assert!(!json.contains("repeated_calls"), "{json}");
    state.shutdown();
}

#[test]
fn commands_refuse_unknown_zones_and_invalid_report_windows_before_answering() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    seed(&root.path().join("data/xtrace.db"));
    let state = live(root.path());
    for zone in ["", "Mars/Olympus_Mons", "system-local"] {
        assert!(matches!(
            state.session_warnings(&["s-warned".into()], zone),
            Err(xtrace_desktop::state::StateError::InvalidTimezone)
        ));
        assert!(matches!(
            state.window_warnings(7, ms(NOW), zone),
            Err(xtrace_desktop::state::StateError::InvalidTimezone)
        ));
    }
    for (days, end) in [(0, ms(NOW)), (2, ms(NOW)), (7, i64::MAX)] {
        assert!(state.window_warnings(days, end, "UTC").is_err());
    }
    assert!(matches!(
        state.dismiss_session_warning("s-clean", "correction:w-h1"),
        Err(xtrace_desktop::state::StateError::UnknownWarning)
    ));
    state.shutdown();
}

#[test]
fn an_admitted_mistake_is_worded_once_and_dismissed_like_one_event() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("data")).unwrap();
    let path = root.path().join("data/xtrace.db");
    {
        let mut store = Store::open(&path).unwrap();
        let text = |uuid: &str, time: &str, words: &str| {
            record(json!({"uuid":uuid,"type":"assistant","timestamp":time,
                "message":{"role":"assistant","content":[{"type":"text","text":words}]}}))
        };
        // Metadata only: the warning reads the saved phrase names.
        session(
            &mut store,
            "s-admitted",
            "claude",
            "Admission",
            &[
                says(
                    "m-h1",
                    "2026-09-07T10:00:00Z",
                    "The parser drops the last line",
                ),
                text(
                    "m-a1",
                    "2026-09-07T10:01:00Z",
                    "You're right, I misread the loop.",
                ),
            ],
            false,
        );
    }
    let state = live(root.path());
    let read = || {
        state
            .session_warnings(&["s-admitted".into()], "UTC")
            .unwrap()
    };
    let list = read();
    let SessionWarningsAnswer::Read {
        warnings, checks, ..
    } = &list.sessions[0].answer
    else {
        panic!("read");
    };
    assert_eq!(warnings.len(), 1);
    let warning = &warnings[0];
    assert_eq!(warning.id, "admitted_mistake:m-a1");
    assert_eq!(warning.label, "Admitted mistake");
    assert_eq!(
        warning.reason,
        "the agent replied \"you're right\" after your message (10:01)"
    );
    assert_eq!(warning.compact_reason, "Agent said \"you're right\"");
    assert_eq!(
        warning.name,
        "Warning. Sep 7, Admitted mistake: the agent replied \"you're right\" after your message (10:01)"
    );
    assert_eq!(warning.record_uuids, vec!["m-h1", "m-a1"]);
    assert_eq!(warning.points_at.record_uuid, "m-a1");
    assert_eq!(warning.points_at.block, None);
    assert_eq!(warning.returned, None);
    assert_eq!(
        checks.last().map(|c| (c.complete, c.text.as_str())),
        Some((true, "Checked: admitted mistakes"))
    );
    // One event: dismissed with no size, and stays dismissed.
    state
        .dismiss_session_warning("s-admitted", "admitted_mistake:m-a1")
        .unwrap();
    let after = read();
    let (active, dismissed, count, _) = shown(&after.sessions[0].answer);
    assert!(active.is_empty());
    assert_eq!(dismissed, vec!["admitted_mistake:m-a1"]);
    assert_eq!(count, None);
    let size: Option<i64> = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT size FROM dismissed_warnings WHERE warning_id='admitted_mistake:m-a1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(size, None);
    state
        .restore_session_warning("s-admitted", "admitted_mistake:m-a1")
        .unwrap();
    let undone = read();
    let (active, dismissed, _, _) = shown(&undone.sessions[0].answer);
    assert_eq!(active, vec!["admitted_mistake:m-a1"]);
    assert!(dismissed.is_empty());
    state.shutdown();
}
