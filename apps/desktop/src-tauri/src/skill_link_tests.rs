//! "Add from a link" against a local test server: nothing here reaches
//! GitHub or any other host.
use super::*;
use crate::grading_link_dto::GradingLinkKeptResult;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

/// Each request the test server saw: its line and its headers.
type Seen = Mutex<Vec<(String, Vec<(String, String)>)>>;
/// One archive case: the archive, the link's suffix, the answer, the bounds.
type Case = (
    Vec<u8>,
    &'static str,
    GradingLinkLookup,
    Box<dyn Fn(&mut LookupLimits)>,
);

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const TOP: &str = "octo-skills-0123456";

fn rubric_text(id: &str) -> String {
    format!(
        "---\nprotocol: 1\nid: {id}\nname: {id} check\nversion: 1\n\
         description: Checks a project's {id} use.\ntarget: project\n---\n\
         ## Uses it well (100 points)\nAll.\n"
    )
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nDo the thing.\n")
}

/// A gzip-compressed archive laid out like GitHub's: one top folder, the
/// files under it, and `links` as symbolic links.
fn archive(files: &[(&str, String)], links: &[(&str, &str)]) -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut top = tar::Header::new_gnu();
    top.set_entry_type(tar::EntryType::Directory);
    top.set_mode(0o755);
    top.set_size(0);
    builder
        .append_data(&mut top, format!("{TOP}/"), std::io::empty())
        .unwrap();
    for (path, text) in files {
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(text.len() as u64);
        builder
            .append_data(&mut header, format!("{TOP}/{path}"), text.as_bytes())
            .unwrap();
    }
    for (path, target) in links {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_mode(0o777);
        header.set_size(0);
        builder
            .append_link(&mut header, format!("{TOP}/{path}"), target)
            .unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// A skills repository: a ready skill, one without a rubric, one with a
/// broken rubric, and two whose names are taken on the test home.
fn skills_repo() -> Vec<u8> {
    archive(
        &[
            ("README.md", "# Skills\n".into()),
            (
                "skills/pdf/SKILL.md",
                skill_md("pdf", "Fills and checks PDF forms."),
            ),
            ("skills/pdf/xtrace-rubric.md", rubric_text("pdf")),
            ("skills/pdf/scripts/fill.py", "print('fill')\n".into()),
            (
                "skills/docx/SKILL.md",
                skill_md("docx", "Edits Word documents with tracked changes."),
            ),
            (
                "skills/notes/SKILL.md",
                skill_md("notes", "Keeps meeting notes tidy."),
            ),
            ("skills/notes/xtrace-rubric.md", "no header\n".into()),
            (
                "skills/taken/SKILL.md",
                skill_md("taken", "Has a name already used in ~/.agents/skills."),
            ),
            (
                "skills/claude-taken/SKILL.md",
                skill_md(
                    "claude-taken",
                    "Has a name already used in ~/.claude/skills.",
                ),
            ),
        ],
        &[],
    )
}

/// One canned answer of the test server.
#[derive(Clone)]
enum Reply {
    /// Status, extra headers, body (sent with its length).
    Answer(u16, Vec<(String, String)>, Vec<u8>),
    /// A body sent in chunks, without a length.
    Chunked(Vec<u8>),
    /// Nothing, for longer than any test waits.
    Stall,
}

fn ok(body: impl Into<Vec<u8>>) -> Reply {
    Reply::Answer(200, Vec::new(), body.into())
}

fn status(code: u16) -> Reply {
    Reply::Answer(code, Vec::new(), b"{}".to_vec())
}

/// A local HTTP server answering by path; it records each request's line
/// and headers (lower-cased names).
struct Server {
    url: String,
    requests: Arc<Seen>,
}

impl Server {
    fn start(routes: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let routes = Arc::new(routes);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let seen = seen.clone();
                let routes = routes.clone();
                std::thread::spawn(move || answer(stream, &*routes, &seen));
            }
        });
        Self { url, requests }
    }

    fn paths(&self) -> Vec<String> {
        lock(&self.requests)
            .iter()
            .map(|(line, _)| line.clone())
            .collect()
    }
}

fn answer(stream: TcpStream, routes: &(dyn Fn(&str) -> Reply + Send + Sync), seen: &Seen) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.trim().split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_owned()));
        }
    }
    let request = line.trim().to_owned();
    let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
    lock(seen).push((request, headers));
    let mut stream = stream;
    match routes(&path) {
        Reply::Answer(code, extra, body) => {
            let mut head = format!(
                "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in extra {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
        Reply::Chunked(body) => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            );
            for chunk in body.chunks(1024) {
                let _ = stream.write_all(format!("{:x}\r\n", chunk.len()).as_bytes());
                let _ = stream.write_all(chunk);
                let _ = stream.write_all(b"\r\n");
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        }
        Reply::Stall => std::thread::sleep(Duration::from_secs(60)),
    }
    let _ = stream.flush();
}

/// The usual routes: `HEAD` and `main` name [`SHA`], whose archive is
/// `archive`.
fn github(archive: Vec<u8>) -> Server {
    Server::start(move |path| match path {
        "/repos/octo/skills/commits/HEAD" | "/repos/octo/skills/commits/main" => ok(SHA),
        p if p == format!("/octo/skills/tar.gz/{SHA}") => ok(archive.clone()),
        _ => status(404),
    })
}

/// A synthetic home with `~/.claude`, one name taken in each skills folder.
fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".agents/skills/taken")).unwrap();
    std::fs::create_dir_all(home.path().join(".claude/skills/claude-taken")).unwrap();
    home
}

struct Setup {
    service: SkillLinkService,
    home: tempfile::TempDir,
    data: tempfile::TempDir,
    changed: Arc<AtomicUsize>,
}

fn setup_with(server: &Server, limits: impl FnOnce(&mut LookupLimits)) -> Setup {
    setup_config(server, |config| limits(&mut config.limits))
}

fn setup_config(server: &Server, edit: impl FnOnce(&mut LinkConfig)) -> Setup {
    let home = home();
    let data = tempfile::tempdir().unwrap();
    let mut config = LinkConfig::new(home.path().to_path_buf(), data.path());
    config.endpoints = Endpoints {
        api: server.url.clone(),
        codeload: server.url.clone(),
    };
    config.system_proxy = false;
    config.clock = Arc::new(|| 1_791_193_200_000);
    config.limits.connect = Duration::from_secs(5);
    config.limits.commit = Duration::from_secs(5);
    config.limits.archive = Duration::from_secs(5);
    edit(&mut config);
    let changed = Arc::new(AtomicUsize::new(0));
    let count = changed.clone();
    let service = SkillLinkService::new(
        config,
        Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
        }),
    );
    Setup {
        service,
        home,
        data,
        changed,
    }
}

fn setup(server: &Server) -> Setup {
    setup_with(server, |_| {})
}

fn look(setup: &Setup, link: &str) -> GradingLinkLookup {
    tauri::async_runtime::block_on(setup.service.lookup(link))
}

fn found(answer: GradingLinkLookup) -> GradingLinkFound {
    match answer {
        GradingLinkLookup::Found(found) => found,
        other => panic!("not found: {other:?}"),
    }
}

/// The look-ups' staging folders left.
fn staged(setup: &Setup) -> usize {
    std::fs::read_dir(setup.data.path().join(skill_link::STAGING_DIR))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn a_repository_link_resolves_the_default_branch_and_previews_every_skill() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let found = found(look(&setup, "https://github.com/octo/skills"));
    assert_eq!(
        server.paths(),
        [
            "GET /repos/octo/skills/commits/HEAD HTTP/1.1".to_owned(),
            format!("GET /octo/skills/tar.gz/{SHA} HTTP/1.1"),
        ]
    );
    // Unauthenticated, named, asking for the bare commit ID.
    let requests = lock(&server.requests).clone();
    let header = |at: usize, name: &str| {
        requests[at]
            .1
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    assert_eq!(
        header(0, "accept").as_deref(),
        Some("application/vnd.github.sha")
    );
    assert_eq!(header(0, "user-agent").as_deref(), Some(USER_AGENT));
    assert_eq!(header(0, "authorization"), None);
    assert_eq!(header(1, "authorization"), None);
    assert_eq!(
        found.source,
        GradingLinkSource {
            repository: "github.com/octo/skills".into(),
            path: String::new(),
            reference: None,
            commit: SHA.into(),
            short_commit: "0123456".into(),
        }
    );
    let rows: Vec<(&str, &GradingLinkKeepState)> = found
        .skills
        .iter()
        .map(|skill| (skill.path.as_str(), &skill.keep))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "skills/claude-taken",
                &GradingLinkKeepState::NameInClaude {
                    folder: "~/.claude/skills/claude-taken".into()
                }
            ),
            ("skills/docx", &GradingLinkKeepState::Keepable),
            ("skills/notes", &GradingLinkKeepState::Keepable),
            ("skills/pdf", &GradingLinkKeepState::Keepable),
            (
                "skills/taken",
                &GradingLinkKeepState::NameInAgents {
                    folder: "~/.agents/skills/taken".into()
                }
            ),
        ]
    );
    assert_eq!(found.keepable, 3);
    let pdf = &found.skills[3];
    assert_eq!(pdf.name, "pdf");
    assert_eq!(pdf.description, "Fills and checks PDF forms.");
    assert_eq!(pdf.files, 3);
    assert_eq!(
        pdf.rubric,
        GradingInstalledRubric::Ready { id: "pdf".into() }
    );
    assert_eq!(pdf.folder_name.as_deref(), Some("pdf"));
    assert_eq!(found.skills[1].rubric, GradingInstalledRubric::Missing);
    assert!(matches!(
        found.skills[2].rubric,
        GradingInstalledRubric::Problem { .. }
    ));
    // The preview is staged privately until it is kept or dropped.
    assert_eq!(staged(&setup), 1);
    use std::os::unix::fs::PermissionsExt;
    let staging = setup.data.path().join(skill_link::STAGING_DIR);
    assert_eq!(
        std::fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn a_folder_link_resolves_its_ref_and_lists_only_that_skill() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let found = found(look(
        &setup,
        "https://github.com/octo/skills/blob/main/skills/pdf/SKILL.md",
    ));
    assert_eq!(
        server.paths()[0],
        "GET /repos/octo/skills/commits/main HTTP/1.1"
    );
    assert_eq!(found.source.reference.as_deref(), Some("main"));
    assert_eq!(found.source.path, "skills/pdf");
    assert_eq!(found.skills.len(), 1);
    assert_eq!(found.skills[0].path, "skills/pdf");
    // Only that folder was unpacked.
    let lookup = std::fs::read_dir(setup.data.path().join(skill_link::STAGING_DIR))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(lookup.join("skills/pdf/SKILL.md").is_file());
    assert!(!lookup.join("skills/docx").exists());
    assert!(!lookup.join("README.md").exists());
}

#[test]
fn links_that_are_not_public_github_ones_send_nothing() {
    let server = github(skills_repo());
    let setup = setup(&server);
    for link in [
        "https://gitlab.com/octo/skills",
        "https://github.com/octo/skills/issues/1",
        "not a link",
    ] {
        assert_eq!(
            look(&setup, link),
            GradingLinkLookup::NotGithubLink,
            "{link}"
        );
    }
    assert!(server.paths().is_empty());
}

#[test]
fn githubs_refusals_are_named() {
    let limited = vec![("X-RateLimit-Remaining".to_owned(), "0".to_owned())];
    for (reply, expected) in [
        (status(404), GradingLinkLookup::NotFound),
        (status(422), GradingLinkLookup::NotFound),
        (status(401), GradingLinkLookup::NotFound),
        (status(403), GradingLinkLookup::NotFound),
        (
            Reply::Answer(403, limited.clone(), b"{}".to_vec()),
            GradingLinkLookup::RateLimited,
        ),
        (status(429), GradingLinkLookup::RateLimited),
        (status(500), GradingLinkLookup::NetworkFailed),
        (ok("not a commit"), GradingLinkLookup::NetworkFailed),
        (ok("x".repeat(1000)), GradingLinkLookup::NetworkFailed),
    ] {
        let reply = Arc::new(reply);
        let server = Server::start(move |_| (*reply).clone());
        let setup = setup(&server);
        assert_eq!(look(&setup, "https://github.com/octo/skills"), expected);
        assert_eq!(staged(&setup), 0);
    }
    // The archive host's own refusal.
    let server = Server::start(|path| match path {
        "/repos/octo/skills/commits/HEAD" => ok(SHA),
        _ => status(404),
    });
    let setup = setup(&server);
    assert_eq!(
        look(&setup, "https://github.com/octo/skills"),
        GradingLinkLookup::NotFound
    );
}

#[test]
fn redirects_are_followed_only_on_the_same_host() {
    let archive = skills_repo();
    let server = Server::start(move |path| match path {
        // A renamed repository answers with its new address.
        "/repos/octo/skills/commits/HEAD" => Reply::Answer(
            301,
            vec![("Location".into(), "/repositories/7/commits/HEAD".into())],
            Vec::new(),
        ),
        "/repositories/7/commits/HEAD" => ok(SHA),
        p if p == format!("/octo/skills/tar.gz/{SHA}") => ok(archive.clone()),
        _ => status(404),
    });
    let same = setup(&server);
    assert!(matches!(
        look(&same, "https://github.com/octo/skills"),
        GradingLinkLookup::Found(_)
    ));
    let elsewhere = Server::start(|_| ok(SHA));
    let away = format!(
        "{}/repositories/7/commits/HEAD",
        elsewhere.url.replace("127.0.0.1", "localhost")
    );
    let server = Server::start(move |_| {
        Reply::Answer(301, vec![("Location".into(), away.clone())], Vec::new())
    });
    let setup = setup(&server);
    assert_eq!(
        look(&setup, "https://github.com/octo/skills"),
        GradingLinkLookup::NetworkFailed
    );
    assert!(elsewhere.paths().is_empty());
}

#[test]
fn downloads_past_the_cap_are_refused_with_or_without_a_length() {
    let big = vec![b'x'; 4096];
    for reply in [ok(big.clone()), Reply::Chunked(big.clone())] {
        let reply = Arc::new(reply);
        let server = Server::start(move |path| match path {
            "/repos/octo/skills/commits/HEAD" => ok(SHA),
            _ => (*reply).clone(),
        });
        let setup = setup_with(&server, |limits| limits.download_bytes = 1024);
        assert_eq!(
            look(&setup, "https://github.com/octo/skills"),
            GradingLinkLookup::TooLarge {
                limit: GradingLinkLimit::Download
            }
        );
        assert_eq!(staged(&setup), 0);
    }
}

#[test]
fn archive_problems_are_named_and_leave_no_staging() {
    let names: Vec<String> = (0..20).map(|at| format!("f{at}")).collect();
    let many: Vec<(&str, String)> = names
        .iter()
        .map(|name| (name.as_str(), "x".to_owned()))
        .collect();
    let cases: Vec<Case> = vec![
        (
            b"not an archive".to_vec(),
            "",
            GradingLinkLookup::BadArchive,
            Box::new(|_| {}),
        ),
        (
            archive(
                &[("README.md", "x".into())],
                &[("skills/a/up", "/etc/passwd")],
            ),
            "",
            GradingLinkLookup::UnsafeArchive,
            Box::new(|_| {}),
        ),
        (
            archive(&[("README.md", "x".into())], &[]),
            "",
            GradingLinkLookup::NoSkill,
            Box::new(|_| {}),
        ),
        (
            skills_repo(),
            "/tree/main/skills/nothing-here",
            GradingLinkLookup::PathNotFound,
            Box::new(|_| {}),
        ),
        (
            skills_repo(),
            "/tree/main/README.md",
            GradingLinkLookup::PathNotFound,
            Box::new(|_| {}),
        ),
        (
            archive(&many, &[]),
            "",
            GradingLinkLookup::TooLarge {
                limit: GradingLinkLimit::Entries,
            },
            Box::new(|limits| limits.archive_limits.entries = 10),
        ),
        (
            archive(&[("big", "x".repeat(5000))], &[]),
            "",
            GradingLinkLookup::TooLarge {
                limit: GradingLinkLimit::Unpacked,
            },
            Box::new(|limits| limits.archive_limits.bytes = 1000),
        ),
    ];
    for (archive, suffix, expected, limits) in cases {
        let server = github(archive);
        let setup = setup_with(&server, limits);
        assert_eq!(
            look(&setup, &format!("https://github.com/octo/skills{suffix}")),
            expected,
            "{suffix}"
        );
        assert_eq!(staged(&setup), 0, "{suffix}");
    }
}

#[test]
fn a_slow_github_times_out() {
    let server = Server::start(|_| Reply::Stall);
    let setup = setup_with(&server, |limits| limits.commit = Duration::from_millis(300));
    let started = Instant::now();
    assert_eq!(
        look(&setup, "https://github.com/octo/skills"),
        GradingLinkLookup::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // The archive's own limit, too.
    let server = Server::start(|path| match path {
        "/repos/octo/skills/commits/HEAD" => ok(SHA),
        _ => Reply::Stall,
    });
    let setup = setup_with(&server, |limits| {
        limits.archive = Duration::from_millis(300)
    });
    assert_eq!(
        look(&setup, "https://github.com/octo/skills"),
        GradingLinkLookup::TimedOut
    );
    assert_eq!(staged(&setup), 0);
}

#[test]
fn cancel_stops_a_request_at_once() {
    for stall_commit in [true, false] {
        let server = Server::start(move |path| match path {
            "/repos/octo/skills/commits/HEAD" if !stall_commit => ok(SHA),
            _ => Reply::Stall,
        });
        let setup = Arc::new(setup_with(&server, |limits| {
            limits.commit = Duration::from_secs(30);
            limits.archive = Duration::from_secs(30);
        }));
        assert!(!setup.service.cancel(), "nothing is going yet");
        let going = setup.clone();
        let started = Instant::now();
        let answer = std::thread::spawn(move || look(&going, "https://github.com/octo/skills"));
        let wanted = if stall_commit { 1 } else { 2 };
        while server.paths().len() < wanted {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(setup.service.cancel());
        assert_eq!(answer.join().unwrap(), GradingLinkLookup::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(staged(&setup), 0);
        assert!(!setup.service.cancel());
    }
}

#[test]
fn a_new_lookup_replaces_the_previous_preview() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let first = found(look(&setup, "https://github.com/octo/skills"));
    let second = found(look(
        &setup,
        "https://github.com/octo/skills/tree/main/skills/pdf",
    ));
    assert_ne!(first.lookup_id, second.lookup_id);
    assert_eq!(staged(&setup), 1);
    assert_eq!(
        setup
            .service
            .keep(&first.lookup_id, &["skills/docx".to_owned()]),
        GradingLinkKeep::Expired
    );
    // Discarding drops it.
    assert!(!setup.service.discard(&first.lookup_id));
    assert!(setup.service.discard(&second.lookup_id));
    assert_eq!(staged(&setup), 0);
    assert_eq!(
        setup
            .service
            .keep(&second.lookup_id, &["skills/pdf".to_owned()]),
        GradingLinkKeep::Expired
    );
}

#[test]
fn keeping_puts_chosen_skills_in_agents_links_them_and_records_the_source() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let preview = found(look(&setup, "https://github.com/octo/skills/tree/main"));
    let answer = setup.service.keep(
        &preview.lookup_id,
        &["skills/pdf".to_owned(), "skills/docx".to_owned()],
    );
    assert_eq!(
        answer,
        GradingLinkKeep::Done {
            skills: vec![
                GradingLinkKept {
                    path: "skills/docx".into(),
                    name: "docx".into(),
                    result: GradingLinkKeptResult::Kept {
                        folder: "~/.agents/skills/docx".into(),
                        claude: GradingLinkClaude::Linked {
                            link: "~/.claude/skills/docx".into()
                        },
                    },
                },
                GradingLinkKept {
                    path: "skills/pdf".into(),
                    name: "pdf".into(),
                    result: GradingLinkKeptResult::Kept {
                        folder: "~/.agents/skills/pdf".into(),
                        claude: GradingLinkClaude::Linked {
                            link: "~/.claude/skills/pdf".into()
                        },
                    },
                },
            ],
            recorded: true,
        }
    );
    let home = setup.home.path();
    assert!(home.join(".agents/skills/pdf/scripts/fill.py").is_file());
    assert_eq!(
        std::fs::read_link(home.join(".claude/skills/pdf")).unwrap(),
        home.join(".agents/skills/pdf")
    );
    // Where each came from, beside XTrace's data, not in the skill.
    let records = LinkedSkills::new(setup.data.path().join(skill_link::LINKED_SKILLS_FILE)).read();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1],
        LinkedSkill {
            folder: home
                .join(".agents/skills/pdf")
                .to_string_lossy()
                .into_owned(),
            claude_link: Some(
                home.join(".claude/skills/pdf")
                    .to_string_lossy()
                    .into_owned()
            ),
            repository: "octo/skills".into(),
            path: "skills/pdf".into(),
            reference: Some("main".into()),
            commit: SHA.into(),
            fetched_at_ms: 1_791_193_200_000,
            kept_at_ms: 1_791_193_200_000,
        }
    );
    assert!(
        !home
            .join(".agents/skills/pdf")
            .join(skill_link::LINKED_SKILLS_FILE)
            .exists()
    );
    // The staging folder is gone, the change was published once, and the
    // preview is used up.
    assert_eq!(staged(&setup), 0);
    assert_eq!(setup.changed.load(Ordering::SeqCst), 1);
    assert_eq!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/notes".to_owned()]),
        GradingLinkKeep::Expired
    );
    // The installed list now has them: pdf ready, docx without a rubric.
    let installed = xt_judge::installed::discover(home);
    let listed: Vec<(&str, bool)> = installed
        .skills
        .iter()
        .map(|skill| {
            (
                skill.name.as_str(),
                matches!(skill.rubric, RubricStatus::Ready(_)),
            )
        })
        .collect();
    assert!(listed.contains(&("pdf", true)));
    assert!(listed.contains(&("docx", false)));
}

#[test]
fn a_name_taken_after_the_preview_keeps_nothing_and_answers_the_fresh_preview() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    std::fs::create_dir(setup.home.path().join(".agents/skills/pdf")).unwrap();
    let chosen = ["skills/pdf".to_owned(), "skills/docx".to_owned()];
    let GradingLinkKeep::Changed(fresh) = setup.service.keep(&preview.lookup_id, &chosen) else {
        panic!("changed");
    };
    let pdf = fresh
        .skills
        .iter()
        .find(|skill| skill.path == "skills/pdf")
        .unwrap();
    assert_eq!(
        pdf.keep,
        GradingLinkKeepState::NameInAgents {
            folder: "~/.agents/skills/pdf".into()
        }
    );
    assert!(!setup.home.path().join(".agents/skills/docx").exists());
    assert_eq!(setup.changed.load(Ordering::SeqCst), 0);
    // The preview is still there: the rest can be kept.
    assert_eq!(staged(&setup), 1);
    assert!(matches!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/docx".to_owned()]),
        GradingLinkKeep::Done { .. }
    ));
    // A path the preview does not have, or none, keeps nothing.
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    assert!(matches!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/elsewhere".to_owned()]),
        GradingLinkKeep::Changed(_)
    ));
    assert_eq!(
        setup.service.keep(&preview.lookup_id, &[]),
        GradingLinkKeep::NothingChosen
    );
}

#[test]
fn an_agents_folder_that_is_a_link_keeps_nothing() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    let elsewhere = tempfile::tempdir().unwrap();
    let agents = setup.home.path().join(".agents/skills");
    std::fs::remove_dir_all(&agents).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), &agents).unwrap();
    assert_eq!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/pdf".to_owned()]),
        GradingLinkKeep::AgentsFolderUnusable
    );
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
}

#[test]
fn quitting_and_starting_again_leave_no_staging() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    setup.service.shutdown();
    assert_eq!(staged(&setup), 0);
    assert_eq!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/pdf".to_owned()]),
        GradingLinkKeep::Expired
    );
    assert_eq!(
        look(&setup, "https://github.com/octo/skills"),
        GradingLinkLookup::Failed
    );
    // A crash's leftovers are removed when the service starts.
    let leftover = setup.data.path().join(skill_link::STAGING_DIR).join("old");
    std::fs::create_dir_all(&leftover).unwrap();
    // So are copies a crash left mid-keep; an empty claimed name stays.
    let agents = setup.home.path().join(".agents/skills");
    let copy = agents.join(".xtrace-keeping-0f8fad5b-0");
    std::fs::create_dir_all(copy.join("scripts")).unwrap();
    std::fs::create_dir(agents.join("claimed")).unwrap();
    let _again = SkillLinkService::new(
        LinkConfig::new(setup.home.path().to_path_buf(), setup.data.path()),
        Arc::new(|| {}),
    );
    assert!(!leftover.exists());
    assert!(!copy.exists());
    assert!(agents.join("claimed").is_dir());
    assert!(agents.join("taken").is_dir());
}

/// A clock that, once armed, holds the next keep that reads it (after the
/// keep has taken the preview, before it copies anything) until released.
#[derive(Clone, Default)]
struct HeldClock(Arc<Mutex<Option<Hold>>>);

/// Where a held read says it arrived, and what lets it go.
type Hold = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

impl HeldClock {
    fn clock(&self) -> Clock {
        let hold = self.0.clone();
        Arc::new(move || {
            let held = lock(&hold).take();
            if let Some((entered, release)) = held {
                entered.send(()).unwrap();
                let _ = release.recv();
            }
            1_791_193_200_000
        })
    }

    /// Arm it: the next read says so on the first channel and waits for the
    /// second.
    fn arm(&self) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (entered_tx, entered) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        *lock(&self.0) = Some((entered_tx, release_rx));
        (entered, release)
    }
}

/// The hidden `.xtrace-keeping-…` copies in `~/.agents/skills`.
fn keeping_copies(setup: &Setup) -> usize {
    std::fs::read_dir(setup.home.path().join(".agents/skills"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(skill_link::KEEP_PREFIX)
        })
        .count()
}

#[test]
fn quitting_during_a_keep_waits_for_it_and_claims_no_name() {
    let server = github(skills_repo());
    let held = HeldClock::default();
    let setup = setup_config(&server, |config| config.clock = held.clock());
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    let (entered, release) = held.arm();
    let service = setup.service.clone();
    let id = preview.lookup_id.clone();
    let keeping = std::thread::spawn(move || service.keep(&id, &["skills/pdf".to_owned()]));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    // Quit while the keep holds the preview.
    let service = setup.service.clone();
    let quitting = std::thread::spawn(move || service.shutdown());
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !quitting.is_finished(),
        "quitting must wait for the keep going"
    );
    release.send(()).unwrap();
    let kept = keeping.join().unwrap();
    quitting.join().unwrap();
    // The keep saw the quit before claiming a name: nothing is in
    // ~/.agents/skills, no hidden copy or staging folder is left, and
    // nothing was recorded or published.
    let agents = setup.home.path().join(".agents/skills");
    assert!(!agents.join("pdf").exists());
    assert_eq!(keeping_copies(&setup), 0);
    assert_eq!(staged(&setup), 0);
    assert_eq!(
        kept,
        GradingLinkKeep::Done {
            skills: vec![GradingLinkKept {
                path: "skills/pdf".into(),
                name: "pdf".into(),
                result: GradingLinkKeptResult::Failed,
            }],
            recorded: false,
        }
    );
    assert_eq!(setup.changed.load(Ordering::SeqCst), 0);
    assert!(
        !setup
            .data
            .path()
            .join(skill_link::LINKED_SKILLS_FILE)
            .exists()
    );
}

#[test]
fn quitting_waits_for_a_keep_only_within_its_bound() {
    let server = github(skills_repo());
    let held = HeldClock::default();
    let setup = setup_config(&server, |config| {
        config.clock = held.clock();
        config.shutdown_bound = Duration::from_millis(200);
    });
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    let (entered, release) = held.arm();
    let service = setup.service.clone();
    let id = preview.lookup_id.clone();
    let keeping = std::thread::spawn(move || service.keep(&id, &["skills/pdf".to_owned()]));
    entered.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    assert!(!setup.service.shutdown(), "the wait ran out");
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(200), "{waited:?}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    // The keep, let go later, still claims nothing and cleans up after itself.
    release.send(()).unwrap();
    keeping.join().unwrap();
    assert!(!setup.home.path().join(".agents/skills/pdf").exists());
    assert_eq!(keeping_copies(&setup), 0);
    assert_eq!(staged(&setup), 0);
}

#[test]
fn fixture_mode_answers_not_configured() {
    let service = SkillLinkService::not_configured();
    assert_eq!(
        tauri::async_runtime::block_on(service.lookup("https://github.com/octo/skills")),
        GradingLinkLookup::NotConfigured
    );
    assert_eq!(
        service.keep("x", &["a".to_owned()]),
        GradingLinkKeep::NotConfigured
    );
    assert!(!service.cancel());
    assert!(!service.discard("x"));
}

/// The answers the renderer's tests and acceptance pages read for "Add from
/// a link". Repositories, names and the home are made up.
#[derive(serde::Serialize)]
struct Export {
    one: GradingLinkLookup,
    several: GradingLinkLookup,
    kept_one: GradingLinkKeep,
    kept_several: GradingLinkKeep,
    changed: GradingLinkKeep,
}

const EXPORT: &str = "../ui/src/app/scores/link.synthetic.json";
const LOOKUP_ID: &str = "00000000-0000-4000-8000-0000000000c1";

fn several_repo() -> Vec<u8> {
    archive(
        &[
            (
                "skills/api-review/SKILL.md",
                skill_md(
                    "api-review",
                    "Reviews a project's public API for breaking changes.",
                ),
            ),
            (
                "skills/api-review/xtrace-rubric.md",
                rubric_text("api-review"),
            ),
            (
                "skills/changelog/SKILL.md",
                skill_md(
                    "changelog",
                    "Checks that the changelog covers every release.",
                ),
            ),
            (
                "skills/docs-check/SKILL.md",
                skill_md("docs-check", "Finds docs that no longer match the code."),
            ),
            ("skills/docs-check/xtrace-rubric.md", "no header\n".into()),
            (
                "skills/taken/SKILL.md",
                skill_md("taken", "Shares a name with a skill you have."),
            ),
            (
                "skills/claude-taken/SKILL.md",
                skill_md("claude-taken", "Shares a name with a Claude Code skill."),
            ),
            (
                "skills/test-plan/SKILL.md",
                skill_md("test-plan", "Writes a test plan for a change."),
            ),
            ("skills/test-plan/notes.md", "x".repeat(2_048)),
        ],
        &[],
    )
}

/// The preview of `archive` at `path`, as the service answers it.
fn previewed(archive: &[u8], link: &str) -> (tempfile::TempDir, Unpacked, Found, GithubLink) {
    let link = skill_link::parse(link).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let into = temp.path().join("stage");
    std::fs::create_dir(&into).unwrap();
    let unpacked = skill_link::unpack(
        archive,
        &into,
        &link.path,
        ArchiveLimits::default(),
        &CancelFlag::new(),
    )
    .unwrap();
    let found = skill_link::find(&unpacked, &link.path, &link.repo).unwrap();
    (temp, unpacked, found, link)
}

fn export() -> Export {
    let home = home();
    let (_one_temp, one_unpacked, one_found, one_link) = previewed(
        &skills_repo(),
        "https://github.com/octo/skills/tree/main/skills/pdf",
    );
    let one = preview(LOOKUP_ID, &one_link, SHA, &one_found, home.path());
    let (_temp, unpacked, several_found, several_link) =
        previewed(&several_repo(), "https://github.com/octo/team-skills");
    let several = preview(LOOKUP_ID, &several_link, SHA, &several_found, home.path());
    // Kept as the service keeps them, into the made-up home.
    let agents = skill_link::agents_folder(home.path()).unwrap();
    let keep = |unpacked: &Unpacked, found: &Found, paths: &[&str]| -> Vec<GradingLinkKept> {
        found
            .skills
            .iter()
            .filter(|skill| paths.contains(&skill.path.as_str()))
            .enumerate()
            .map(|(n, skill)| GradingLinkKept {
                path: skill.path.clone(),
                name: skill.name.clone(),
                result: kept_dto(
                    skill_link::keep(unpacked, skill, home.path(), &agents, &format!("e{n}")),
                    home.path(),
                ),
            })
            .collect()
    };
    let kept_one = GradingLinkKeep::Done {
        skills: keep(&one_unpacked, &one_found, &["skills/pdf"]),
        recorded: true,
    };
    // ~/.claude/skills/changelog is taken: it is kept without the link.
    std::fs::create_dir(home.path().join(".claude/skills/changelog")).unwrap();
    let kept_several = GradingLinkKeep::Done {
        skills: keep(
            &unpacked,
            &several_found,
            &["skills/api-review", "skills/changelog"],
        ),
        recorded: true,
    };
    // After those, the same preview again: their names are taken now.
    let changed = GradingLinkKeep::Changed(preview(
        LOOKUP_ID,
        &several_link,
        SHA,
        &several_found,
        home.path(),
    ));
    Export {
        one: GradingLinkLookup::Found(one),
        several: GradingLinkLookup::Found(several),
        kept_one,
        kept_several,
        changed,
    }
}

/// The committed export is exactly what the rule gives now. Regenerate it
/// with `XTRACE_WRITE_SCORES_SYNTHETIC=1`.
#[test]
fn the_synthetic_ui_export_is_what_the_rule_gives() {
    let export = export();
    let json = serde_json::to_string_pretty(&export).unwrap() + "\n";
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPORT);
    if std::env::var_os("XTRACE_WRITE_SCORES_SYNTHETIC").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    assert_eq!(
        json,
        std::fs::read_to_string(&path).unwrap_or_default(),
        "regenerate with XTRACE_WRITE_SCORES_SYNTHETIC=1"
    );
    // What the renderer's acceptance relies on.
    let GradingLinkLookup::Found(several) = &export.several else {
        panic!("several is found");
    };
    let states: Vec<&GradingLinkKeepState> =
        several.skills.iter().map(|skill| &skill.keep).collect();
    assert!(states.contains(&&GradingLinkKeepState::Keepable));
    assert!(
        states
            .iter()
            .any(|state| matches!(state, GradingLinkKeepState::NameInAgents { .. }))
    );
    assert!(
        states
            .iter()
            .any(|state| matches!(state, GradingLinkKeepState::NameInClaude { .. }))
    );
    assert_eq!(several.keepable, 4);
    let GradingLinkKeep::Done { skills, .. } = &export.kept_several else {
        panic!("done");
    };
    assert!(skills.iter().any(|skill| matches!(
        &skill.result,
        GradingLinkKeptResult::Kept {
            claude: GradingLinkClaude::NameTaken,
            ..
        }
    )));
}

#[test]
fn keeping_moves_an_unusable_records_file_aside_instead_of_overwriting_it() {
    let server = github(skills_repo());
    let setup = setup(&server);
    let records = setup.data.path().join(skill_link::LINKED_SKILLS_FILE);
    std::fs::write(&records, "not json").unwrap();
    let preview = found(look(&setup, "https://github.com/octo/skills"));
    assert!(matches!(
        setup
            .service
            .keep(&preview.lookup_id, &["skills/pdf".to_owned()]),
        GradingLinkKeep::Done { recorded: true, .. }
    ));
    assert_eq!(
        std::fs::read_to_string(setup.data.path().join("linked-skills.json.bad")).unwrap(),
        "not json"
    );
    assert_eq!(LinkedSkills::new(records).read().len(), 1);
}
