use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use xt_store::Store;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
const USAGE: &str = "Usage: xtrace-core serve --db PATH [--port PORT] [--bind 127.0.0.1|::1]\n       xtrace-core import-native --db PATH --home DIR [--host claude|codex|cursor]... [--pin FILE] [--plugin-root DIR] [--python EXE] [--replay]\n       xtrace-core watch-native --db PATH --home DIR [--host claude|codex|cursor]... [--pin FILE] [--plugin-root DIR] [--python EXE] [--once | --for SECONDS]\n       xtrace-core grade --project DIR --out DIR [--skill claude-md-doctor]\n       xtrace-core make-rubric --skill DIR --out DIR [--remake]";

async fn run() -> Result<(), &'static str> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("serve") => {}
        Some("import-native") => return import_native(args),
        Some("watch-native") => return watch_native(args),
        Some("grade") => return grade(args).await,
        Some("make-rubric") => return make_rubric(args).await,
        _ => return Err(USAGE),
    }
    let (mut db, mut port, mut ip) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("A startup option is missing its value")?;
        match flag.to_str() {
            Some("--db") if db.is_none() => db = Some(PathBuf::from(value)),
            Some("--port") if port.is_none() => {
                port = Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<u16>().ok())
                        .ok_or("Invalid server port")?,
                )
            }
            Some("--bind") if ip.is_none() => {
                ip = Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<IpAddr>().ok())
                        .ok_or("Invalid literal bind address")?,
                )
            }
            _ => return Err("Unknown or duplicate startup option"),
        }
    }
    let db = db
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--db PATH is required")?;
    let ip = ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    if ip != IpAddr::V4(Ipv4Addr::LOCALHOST) && ip != IpAddr::V6(std::net::Ipv6Addr::LOCALHOST) {
        return Err("Server must bind a literal localhost address");
    }
    let store = Store::open(&db).map_err(|_| "Could not open the server database")?;
    let configured = if port.is_some() {
        None
    } else {
        store
            .server_port()
            .map_err(|_| "Invalid persisted server.port setting")?
    };
    let listener = xt_server::bind(SocketAddr::new(
        ip,
        port.or(configured).unwrap_or(xt_server::DEFAULT_PORT),
    ))
    .await
    .map_err(|_| "Could not bind the configured loopback port; it may be occupied")?;
    let address = listener
        .local_addr()
        .map_err(|_| "Could not read bound server address")?;
    println!("XTRACE_PORT={}", address.port());
    io::stdout()
        .flush()
        .map_err(|_| "Could not report bound server port")?;
    xt_server::serve(listener, store, db, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| "Loopback server failed")
}

struct NativeOptions {
    db: PathBuf,
    home: PathBuf,
    hosts: Vec<xt_store::Host>,
    pin: PathBuf,
    plugin_root: Option<PathBuf>,
    python: Option<std::ffi::OsString>,
    replay: bool,
    /// `watch-native`: exit once ready (`--once`) or after this many seconds (`--for`).
    duration: Option<Option<u64>>,
}

fn parse_native_options(
    mut args: std::iter::Skip<std::env::ArgsOs>,
    watch: bool,
) -> Result<NativeOptions, &'static str> {
    use xt_store::Host;
    let (mut db, mut home, mut pin, mut plugin_root, mut python) = (None, None, None, None, None);
    let mut hosts = Vec::new();
    let mut replay = false;
    let mut duration: Option<Option<u64>> = None;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--replay") if !watch && !replay => {
                replay = true;
                continue;
            }
            Some("--once") if watch && duration.is_none() => {
                duration = Some(None);
                continue;
            }
            _ => {}
        }
        let value = args.next().ok_or("An import option is missing its value")?;
        match flag.to_str() {
            Some("--db") if db.is_none() => db = Some(PathBuf::from(value)),
            Some("--home") if home.is_none() => home = Some(PathBuf::from(value)),
            Some("--pin") if pin.is_none() => pin = Some(PathBuf::from(value)),
            Some("--plugin-root") if plugin_root.is_none() => {
                plugin_root = Some(PathBuf::from(value))
            }
            Some("--python") if python.is_none() => python = Some(value),
            Some("--for") if watch && duration.is_none() => {
                duration = Some(Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or("--for requires a whole number of seconds")?,
                ))
            }
            Some("--host") => {
                let host = match value.to_str() {
                    Some("claude") => Host::Claude,
                    Some("codex") => Host::Codex,
                    Some("cursor") => Host::Cursor,
                    _ => return Err("--host must be claude, codex or cursor"),
                };
                if hosts.contains(&host) {
                    return Err("Unknown or duplicate import option");
                }
                hosts.push(host);
            }
            _ => return Err("Unknown or duplicate import option"),
        }
    }
    let db = db
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--db PATH is required")?;
    let home = home
        .filter(|p| p.is_dir())
        .ok_or("--home DIR must name an existing directory")?;
    if hosts.is_empty() {
        hosts = vec![Host::Claude, Host::Codex, Host::Cursor];
    }
    let plugin_root =
        plugin_root.or_else(|| std::env::var_os("AGENT_PLUGINS_DIR").map(PathBuf::from));
    // The CLI honors the conventional `PYTHON` variable; the library does not
    // read it, so a desktop process inheriting one is unaffected.
    let python = python.or_else(|| std::env::var_os("PYTHON").filter(|value| !value.is_empty()));
    let pin = pin.unwrap_or_else(|| PathBuf::from(".plugin-pin"));
    Ok(NativeOptions {
        db,
        home,
        hosts,
        pin,
        plugin_root,
        python,
        replay,
        duration,
    })
}

fn clock_ms() -> Result<i64, &'static str> {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Import clock is unavailable")?
            .as_millis(),
    )
    .map_err(|_| "Import clock is unavailable")
}

/// One-shot import of native session history through the pinned shared readers,
/// resuming behind proven checkpoints unless `--replay` is given. Prints a JSON
/// report; exits 0 when every requested host imported completely, 2 when a host
/// or session was unavailable, unreadable or incomplete.
fn import_native(args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use xt_ingest::native::{ImportRequest, ProducerSource, ScanMode, scan_native};
    let options = parse_native_options(args, false)?;
    let observed_at = clock_ms()?;
    xt_ingest::native::validate_index_destination(&options.db, &options.home)?;
    let mut store = Store::open(&options.db).map_err(|_| "Could not open the index database")?;
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &options.home,
            hosts: &options.hosts,
            producer: &ProducerSource::Checkout {
                pin: options.pin.clone(),
                plugin_root: options.plugin_root.clone(),
            },
            python: options.python.as_deref(),
            observed_at,
            cancel: None,
        },
        if options.replay {
            ScanMode::Replay
        } else {
            ScanMode::Resume
        },
    );
    let text =
        serde_json::to_string_pretty(&report).map_err(|_| "Could not encode the import report")?;
    println!("{text}");
    io::stdout()
        .flush()
        .map_err(|_| "Could not write the import report")?;
    if !report.complete() {
        std::process::exit(2);
    }
    Ok(())
}

/// Watch native history and keep the index current: the watcher registers
/// before the initial scan, changes queued meanwhile are reconciled before
/// `ready`, then each burst of changes is reconciled live. Prints one JSON
/// line per event. `--once` exits after `ready`; `--for` exits after the given
/// seconds; otherwise it runs until interrupted. Both bounds cover the startup
/// too: `--for` elapsing or Ctrl-C arriving during the initial scan stops the
/// tailer once that scan is done, reconciling what it received, and the run
/// exits 1 as not ready. Exits 0 when the final freshness is live and every
/// scan was complete, 2 otherwise. An event that cannot be written (the
/// consumer of the stream is gone) stops the tailer and fails the run, so the
/// watcher never runs on with no observable output.
fn watch_native(args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;
    use xt_ingest::native::{
        ProducerSource,
        watch::{Freshness, TailEvent, Tailer, WatchConfig},
    };
    let options = parse_native_options(args, true)?;
    xt_ingest::native::validate_index_destination(&options.db, &options.home)?;
    let store = Store::open(&options.db).map_err(|_| "Could not open the index database")?;
    let complete = Arc::new(AtomicBool::new(true));
    let live = Arc::new(AtomicBool::new(false));
    let output = Arc::new(Mutex::new(io::stdout()));
    let output_failed = Arc::new(AtomicBool::new(false));
    // Ends the wait below: Ctrl-C, or an event that could not be written.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let sink = {
        let (complete, live, output, output_failed, stop) = (
            Arc::clone(&complete),
            Arc::clone(&live),
            Arc::clone(&output),
            Arc::clone(&output_failed),
            stop_tx.clone(),
        );
        Box::new(move |event: TailEvent| {
            // Freshness is taken from every event, so a root that could not
            // be watched after readiness still fails the run.
            match &event {
                TailEvent::Ready(readiness) => {
                    live.store(readiness.freshness == Freshness::Live, Ordering::SeqCst);
                    if !readiness.report.complete() {
                        complete.store(false, Ordering::SeqCst);
                    }
                }
                TailEvent::Reconciled {
                    freshness, report, ..
                } => {
                    live.store(*freshness == Freshness::Live, Ordering::SeqCst);
                    if !report.complete() {
                        complete.store(false, Ordering::SeqCst);
                    }
                }
                TailEvent::Stopped { freshness } => {
                    live.store(*freshness == Freshness::Live, Ordering::SeqCst);
                }
                // Display relations only: neither freshness nor completeness.
                TailEvent::SessionCreationsChanged { .. } => {}
            }
            let written = serde_json::to_string(&event)
                .ok()
                .and_then(|text| {
                    let mut out = output.lock().ok()?;
                    writeln!(out, "{text}").ok()?;
                    out.flush().ok()
                })
                .is_some();
            if !written {
                output_failed.store(true, Ordering::SeqCst);
                let _ = stop.send(());
            }
        }) as Box<dyn Fn(TailEvent) + Send>
    };
    // The bound on the run is armed before the tailer starts, so it covers
    // the startup: `--for` elapsing or Ctrl-C arriving during the initial
    // scan stops the tailer once that scan is done instead of running on or
    // killing the process with changes received and unreconciled.
    match options.duration {
        Some(None) => {}
        Some(Some(seconds)) => {
            let stop = stop_tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                let _ = stop.send(());
            });
        }
        None => ctrlc_wait(stop_tx.clone()),
    }
    let tailer = Tailer::start(
        store,
        WatchConfig {
            usage_probe_dir: None,
            home: options.home.clone(),
            hosts: options.hosts.clone(),
            producer: ProducerSource::Checkout {
                pin: options.pin.clone(),
                plugin_root: options.plugin_root.clone(),
            },
            python: options.python.clone(),
            debounce: Duration::from_millis(250),
            spawn_limits: xt_ingest::native::session_creation::spawn_limits(),
            probe: None,
        },
        sink,
    );
    // Readiness counts only if no stop was queued first: a bound reached
    // while the startup was still running, even within the same wait step,
    // means the run did not become ready within its bound.
    let ready = loop {
        let ready = tailer.wait_ready(Duration::from_millis(200));
        let stop_requested = match stop_rx.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
        };
        if stop_requested || tailer.status().stopped {
            break None;
        }
        if ready.is_some() {
            break ready;
        }
    };
    if ready.is_none() {
        tailer.stop();
        if output_failed.load(Ordering::SeqCst) {
            return Err("Could not write the event stream");
        }
        return Err("Native watcher stopped before it was ready");
    }
    if options.duration != Some(None) {
        let _ = stop_rx.recv();
    }
    tailer.stop();
    if output_failed.load(Ordering::SeqCst) {
        return Err("Could not write the event stream");
    }
    if !(live.load(Ordering::SeqCst) && complete.load(Ordering::SeqCst)) {
        std::process::exit(2);
    }
    Ok(())
}

/// Resolve the sender when Ctrl-C arrives (or the runtime cannot listen).
fn ctrlc_wait(stop: std::sync::mpsc::Sender<()>) {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop.send(());
    });
}

/// Developer command: grade one project folder with a built-in grading skill
/// (bundled under `vendor/` in this checkout), run headless by the user's
/// Claude Code, and print the validated protocol v1 result as JSON. `--out`
/// names the run's own grade folder; it is created when missing and must be
/// empty. It is checked against `--project` before anything is created, so a
/// refused run creates no folder. `XTRACE_CLAUDE` may name the Claude Code executable. Ctrl-C cancels
/// the run, ending Claude Code and everything it started. Exits 0 with a
/// result, 2 when the run produced none (the reason is printed), 1 on a usage
/// error. Not wired into the app.
async fn grade(mut args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use xt_judge::{CancelFlag, GradeRun, GradingSkill, Limits, runner, skill};
    let (mut project, mut skill_id, mut out) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("A grade option is missing its value")?;
        match flag.to_str() {
            Some("--project") if project.is_none() => project = Some(PathBuf::from(value)),
            Some("--out") if out.is_none() => out = Some(PathBuf::from(value)),
            Some("--skill") if skill_id.is_none() => {
                skill_id = Some(value.into_string().map_err(|_| "Unknown grading skill")?)
            }
            _ => return Err("Unknown or duplicate grade option"),
        }
    }
    let project = project
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--project DIR is required")?;
    let out = out
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--out DIR is required")?;
    let skill_id = skill_id.unwrap_or_else(|| skill::BUILTIN[0].to_owned());
    let vendor = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor"));
    let dir = skill::bundled_dir(&vendor, &skill_id).map_err(|_| "Unknown grading skill")?;
    let grading_skill = GradingSkill::open_builtin(&dir, &skill_id)
        .map_err(|_| "The bundled grading skill or its rubric is missing or invalid")?;
    let env = runner::inherited_env();
    let claude = find_claude(&env)?;
    // The runner checks the same rules again once the folder exists; here
    // they run first, so a refused run leaves nothing behind.
    if let Err(error) = runner::check_planned(&project, &out) {
        eprintln!("{error}");
        std::process::exit(2);
    }
    std::fs::create_dir_all(&out).map_err(|_| "Could not create the grade folder")?;
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        runner::run(
            &GradeRun {
                claude: &claude,
                skill: &grading_skill,
                project: &project,
                grade_dir: &out,
                inherited_env: &env,
                limits: Limits::default(),
                leader_record: None,
                step: None,
            },
            Some(&flag),
        )
    });
    let outcome = tokio::select! {
        outcome = &mut task => outcome,
        _ = tokio::signal::ctrl_c() => {
            cancel.cancel();
            task.await
        }
    }
    .map_err(|_| "The grading run stopped unexpectedly")?;
    match outcome {
        Ok(result) => {
            let text = serde_json::to_string_pretty(&result)
                .map_err(|_| "Could not encode the grading result")?;
            println!("{text}");
            io::stdout()
                .flush()
                .map_err(|_| "Could not write the grading result")?;
            Ok(())
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

/// The Claude Code executable a developer command runs: `XTRACE_CLAUDE`, or
/// the shared resolver's search ([`xt_judge::runner::find_claude`]).
fn find_claude(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> Result<PathBuf, &'static str> {
    use xt_judge::runner;
    runner::find_claude(env).map_err(|reason| match reason {
        runner::Unavailable::OverrideNotAbsolute => {
            "XTRACE_CLAUDE must be an absolute path to the Claude Code executable"
        }
        runner::Unavailable::OverrideNotExecutable => {
            "XTRACE_CLAUDE does not name an executable file"
        }
        runner::Unavailable::NotFound => {
            "Claude Code (claude) was not found on PATH or in a standard install folder; \
             set XTRACE_CLAUDE to its absolute path"
        }
        runner::Unavailable::UnsupportedPlatform => "Grading needs a Unix host",
    })
}

/// Developer command: make a rubric for one skill folder exactly as the
/// app's "Make a rubric" job does ([`xt_judge::maker::make`]: the bundled
/// `xtrace-rubric-maker` helper from this checkout, the same staging,
/// sandboxed Claude Code command line, version check, retry, refusal,
/// deadline and version XTrace sets), but save nothing next to the skill or
/// in any rubric store. A made rubric is copied to `<out>/xtrace-rubric.md`,
/// a refusal's reason to `<out>/refusal.txt`. `--out` is created when
/// missing and must be empty; it may neither contain nor be inside the
/// skill folder, checked before anything is created. `--remake` makes again
/// a rubric the skill has that the rubric reader refuses (the app's Remake);
/// without it the skill must have none (the app's Make). `XTRACE_CLAUDE`
/// may name the Claude Code executable. Ctrl-C cancels the job. Prints one
/// JSON object (`outcome` made, refused or failed, `reason`, `id`,
/// `version`, `criteria`) and exits 0 when made or refused, 2 when failed,
/// 1 on a usage error. Not wired into the app.
async fn make_rubric(mut args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use xt_judge::{
        CancelFlag, GradeError, Limits, RubricError,
        maker::{self, MakeError, Written},
        rubric, runner,
    };
    let (mut skill, mut out, mut remake) = (None, None, false);
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--remake") if !remake => remake = true,
            Some("--skill") if skill.is_none() => {
                skill = Some(PathBuf::from(
                    args.next()
                        .ok_or("A make-rubric option is missing its value")?,
                ));
            }
            Some("--out") if out.is_none() => {
                out = Some(PathBuf::from(
                    args.next()
                        .ok_or("A make-rubric option is missing its value")?,
                ));
            }
            _ => return Err("Unknown or duplicate make-rubric option"),
        }
    }
    let skill = skill
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--skill DIR is required")?;
    let out = out
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--out DIR is required")?;
    let skill = std::fs::canonicalize(&skill)
        .ok()
        .filter(|folder| folder.join("SKILL.md").is_file())
        .ok_or("--skill must name a skill folder with a SKILL.md")?;
    // Before anything is created: the output folder is outside the skill.
    match runner::check_planned(&skill, &out) {
        Ok(()) => {}
        Err(GradeError::GradeDirOverlapsProject) => {
            return Err("the output folder and the skill folder must not contain each other");
        }
        Err(_) => return Err("--out must name a usable folder path without whitespace"),
    }
    if std::fs::read_dir(&out).is_ok_and(|mut entries| entries.next().is_some()) {
        return Err("the output folder must be empty");
    }
    // The app's rule: Make for a skill without a rubric, Remake for one
    // whose rubric is refused; a valid rubric is never replaced.
    let own = skill.join(rubric::RUBRIC_FILE);
    let replaced = match (rubric::read(&skill), remake) {
        (Ok(_), _) => return Err("the skill already has a valid xtrace-rubric.md"),
        (Err(RubricError::Missing), false) => None,
        (Err(RubricError::Missing), true) => {
            return Err("the skill has no xtrace-rubric.md to remake; leave out --remake");
        }
        (Err(_), false) => {
            return Err("the skill's xtrace-rubric.md is refused; use --remake to make it again");
        }
        (Err(error), true) => Some(error),
    };
    let helper = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/xt-judge/rubric-maker"
    ));
    let env = runner::inherited_env();
    let claude = find_claude(&env)?;
    std::fs::create_dir_all(&out).map_err(|_| "Could not create the output folder")?;
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let run_id = uuid::Uuid::new_v4().to_string();
        let key = maker::data_key(std::path::Path::new("xtrace-core make-rubric"));
        maker::make(
            &maker::Job {
                claude: &claude,
                temp_dir: &std::env::temp_dir(),
                key: &key,
                run_id: &run_id,
                skill: &skill,
                helper: &helper,
                replaced: replaced.map(|error| maker::Replaced { error, path: &own }),
                inherited_env: &env,
                limits: Limits {
                    deadline: maker::DEFAULT_DEADLINE,
                    ..Limits::default()
                },
                step: None,
            },
            Some(&flag),
        )
    });
    let outcome = tokio::select! {
        outcome = &mut task => outcome,
        _ = tokio::signal::ctrl_c() => {
            cancel.cancel();
            task.await
        }
    }
    .map_err(|_| "The rubric-making job stopped unexpectedly")?;
    let (report, code) = match outcome {
        Ok(Written::Rubric(made)) => {
            std::fs::write(out.join(rubric::RUBRIC_FILE), &made.bytes)
                .map_err(|_| "Could not write the made rubric")?;
            let criteria: Vec<serde_json::Value> = made
                .rubric
                .criteria
                .iter()
                .map(|criterion| {
                    serde_json::json!({"name": criterion.name, "points": criterion.points})
                })
                .collect();
            (
                serde_json::json!({
                    "outcome": "made", "reason": null, "id": made.rubric.id,
                    "version": made.rubric.version, "criteria": criteria, "tries": made.tries,
                }),
                0,
            )
        }
        Ok(Written::Refused(refusal)) => {
            std::fs::write(out.join("refusal.txt"), format!("{}\n", refusal.reason))
                .map_err(|_| "Could not write the refusal")?;
            (
                serde_json::json!({
                    "outcome": "refused", "reason": refusal.reason, "id": null,
                    "version": null, "criteria": [], "tries": refusal.tries,
                }),
                0,
            )
        }
        Err(error) => {
            let reason = match error {
                MakeError::Rubric(RubricError::Missing) => {
                    "Claude wrote neither a rubric nor a refusal, on the retry either".to_owned()
                }
                MakeError::Run(GradeError::Cancelled) => "the job was cancelled".to_owned(),
                other => other.to_string(),
            };
            (
                serde_json::json!({
                    "outcome": "failed", "reason": reason, "id": null,
                    "version": null, "criteria": [], "tries": null,
                }),
                2,
            )
        }
    };
    let text =
        serde_json::to_string_pretty(&report).map_err(|_| "Could not encode the job's outcome")?;
    println!("{text}");
    io::stdout()
        .flush()
        .map_err(|_| "Could not write the job's outcome")?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}
