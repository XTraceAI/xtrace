//! "Add from a link" in the Add a skill dialog: look a public GitHub link
//! up, preview the skills it holds, and keep the chosen ones in
//! `~/.agents/skills` (with a `~/.claude/skills` link when that name is free).
//!
//! * **Network.** Only [`SkillLinkService::lookup`], started by the user's
//!   Look up, uses it, and only GitHub without signing in: one GitHub API
//!   request resolves the link's branch, tag or commit (the default branch
//!   when it names none) to one commit
//!   (`GET /repos/<owner>/<repo>/commits/<ref or HEAD>`, answered as a bare
//!   commit ID), then that commit's archive is downloaded from GitHub's
//!   archive host (`codeload.github.com/<owner>/<repo>/tar.gz/<commit>`), at
//!   most [`xt_judge::skill_link::MAX_DOWNLOAD_BYTES`]. Each request has its
//!   own time limit; a redirect is followed only on the same host, at most
//!   [`MAX_REDIRECTS`] times. Nothing is sent but the request itself.
//! * **One look-up at a time.** A new look-up cancels one going and drops a
//!   finished one's preview. Cancel ([`SkillLinkService::cancel`]) stops the
//!   request going at once, and the unpacking at its next entry.
//! * **Staging.** The archive is unpacked within bounds
//!   ([`xt_judge::skill_link::unpack`]) into a private folder of its own,
//!   `<data>/skill-link-staging/<look-up id>/`, which stays until the
//!   preview is kept, discarded ([`SkillLinkService::discard`]) or replaced,
//!   or XTrace quits ([`SkillLinkService::shutdown`]); a start removes any
//!   left by a crash. Nothing downloaded is run.
//! * **Preview.** The skills found and whether each can be kept are decided
//!   by `xt_judge::skill_link` ([`found_dto`]).
//! * **Keep.** [`SkillLinkService::keep`] decides again over the chosen
//!   skills; when one cannot be kept now, nothing is kept and the fresh
//!   preview is answered. Otherwise each is kept
//!   ([`xt_judge::skill_link::keep`]), where it came from is recorded in
//!   `<data>/linked-skills.json`, the staging folder is removed, and
//!   `grading://changed` follows when anything was kept: the skill then
//!   shows in the installed list like any other.
//!
//! Fixture mode's service is not configured: it answers `not_configured`
//! and never uses the network or reads a home folder.
use crate::grading_dto::GradingInstalledRubric;
use crate::grading_link_dto::{
    GradingLinkClaude, GradingLinkFound, GradingLinkKeep, GradingLinkKeepState, GradingLinkKept,
    GradingLinkKeptResult, GradingLinkLimit, GradingLinkLookup, GradingLinkSkill,
    GradingLinkSource,
};
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::pin,
    sync::{Arc, Mutex, MutexGuard},
    task::{Poll, Waker},
    time::Duration,
};
use xt_judge::{
    CancelFlag,
    installed::RubricStatus,
    skill_link::{
        self, ArchiveError, ArchiveLimits, ClaudeLink, FindError, Found, GithubLink, KeepState,
        Kept, LinkedSkill, LinkedSkills, SkillProblem, Unpacked,
    },
};

/// GitHub's API.
pub const GITHUB_API: &str = "https://api.github.com";
/// GitHub's archive host.
pub const GITHUB_CODELOAD: &str = "https://codeload.github.com";
/// What XTrace calls itself to GitHub (GitHub's API needs a name).
pub const USER_AGENT: &str = "XTrace-Desktop";
/// Most redirects one request follows, each on the same host.
pub const MAX_REDIRECTS: usize = 3;
/// Largest commit answer read.
const MAX_COMMIT_ANSWER: u64 = 256;

/// Called after a keep that kept something.
pub type Publish = Arc<dyn Fn() + Send + Sync>;
/// The wall clock, in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Where the two GitHub hosts are (tests serve them locally).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub api: String,
    pub codeload: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            api: GITHUB_API.to_owned(),
            codeload: GITHUB_CODELOAD.to_owned(),
        }
    }
}

/// The bounds of one look-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookupLimits {
    pub connect: Duration,
    /// The commit request, from connecting to its last byte.
    pub commit: Duration,
    /// The archive download, from connecting to its last byte.
    pub archive: Duration,
    pub download_bytes: u64,
    pub archive_limits: ArchiveLimits,
}

impl Default for LookupLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(15),
            commit: Duration::from_secs(30),
            archive: Duration::from_secs(180),
            download_bytes: skill_link::MAX_DOWNLOAD_BYTES,
            archive_limits: ArchiveLimits::default(),
        }
    }
}

/// Everything a configured service needs.
pub struct LinkConfig {
    /// The native home whose `~/.agents/skills` and `~/.claude/skills` are
    /// used.
    pub home: PathBuf,
    /// `<data>/skill-link-staging`.
    pub staging: PathBuf,
    /// `<data>/linked-skills.json`.
    pub records: PathBuf,
    pub endpoints: Endpoints,
    pub limits: LookupLimits,
    /// Whether the system's proxy settings apply (tests turn them off).
    pub system_proxy: bool,
    pub clock: Clock,
}

impl LinkConfig {
    /// The production settings over the given folders.
    pub fn new(home: PathBuf, data_dir: &Path) -> Self {
        Self {
            home,
            staging: data_dir.join(skill_link::STAGING_DIR),
            records: data_dir.join(skill_link::LINKED_SKILLS_FILE),
            endpoints: Endpoints::default(),
            limits: LookupLimits::default(),
            system_proxy: true,
            clock: Arc::new(now_ms),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A look-up's cancel: the flag the unpacking checks, and the waker of the
/// request waiting now.
#[derive(Default)]
struct Cancel {
    flag: CancelFlag,
    waker: Mutex<Option<Waker>>,
}

impl Cancel {
    fn cancel(&self) {
        self.flag.cancel();
        if let Some(waker) = lock(&self.waker).take() {
            waker.wake();
        }
    }
}

/// `work`, or none as soon as `cancel` is cancelled (`work` is dropped
/// then, which ends its request).
async fn race<T>(work: impl Future<Output = T>, cancel: &Cancel) -> Option<T> {
    let mut work = pin!(work);
    std::future::poll_fn(|cx| {
        *lock(&cancel.waker) = Some(cx.waker().clone());
        if cancel.flag.is_cancelled() {
            return Poll::Ready(None);
        }
        work.as_mut().poll(cx).map(Some)
    })
    .await
}

/// A finished look-up, held for keeping.
struct Lookup {
    id: String,
    link: GithubLink,
    commit: String,
    fetched_at_ms: i64,
    unpacked: Unpacked,
    found: Found,
}

enum Slot {
    Idle,
    Looking {
        id: String,
        cancel: Arc<Cancel>,
    },
    Ready(Lookup),
    /// Its preview is being kept now.
    Keeping {
        id: String,
    },
}

struct Inner {
    config: Option<LinkConfig>,
    changed: Publish,
    /// Held only briefly: never across a request, the unpacking or a keep.
    slot: Mutex<Slot>,
    stopped: std::sync::atomic::AtomicBool,
}

#[derive(Clone)]
pub struct SkillLinkService {
    inner: Arc<Inner>,
}

impl SkillLinkService {
    /// A configured service. Staging folders a crash left are removed now,
    /// and so are hidden `.xtrace-keeping-…` copies left in
    /// `~/.agents/skills` ([`skill_link::remove_keeping_leftovers`]; an
    /// empty folder claimed for a name is left alone); nothing else is read
    /// or sent until asked.
    pub fn new(config: LinkConfig, changed: Publish) -> Self {
        skill_link::remove_all(&config.staging);
        skill_link::remove_keeping_leftovers(&config.home);
        Self::with(Some(config), changed)
    }

    /// Fixture mode: every command answers `not_configured`.
    pub fn not_configured() -> Self {
        Self::with(None, Arc::new(|| {}))
    }

    fn with(config: Option<LinkConfig>, changed: Publish) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                changed,
                slot: Mutex::new(Slot::Idle),
                stopped: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    /// Look `text` up and keep the preview (see the module docs). Another
    /// look-up going or kept is cancelled or dropped first.
    pub async fn lookup(&self, text: &str) -> GradingLinkLookup {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return GradingLinkLookup::NotConfigured;
        };
        let Ok(link) = skill_link::parse(text) else {
            return GradingLinkLookup::NotGithubLink;
        };
        if inner.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return GradingLinkLookup::Failed;
        }
        let Some(id) = crate::telemetry::random_uuid() else {
            return GradingLinkLookup::Failed;
        };
        let cancel = Arc::new(Cancel::default());
        let replaced = std::mem::replace(
            &mut *lock(&inner.slot),
            Slot::Looking {
                id: id.clone(),
                cancel: cancel.clone(),
            },
        );
        drop_slot(replaced);
        let answer = fetch(config, &id, &link, &cancel).await;
        let mut slot = lock(&inner.slot);
        let ours = matches!(&*slot, Slot::Looking { id: going, .. } if *going == id);
        match answer {
            Ok(lookup) if ours && !cancel.flag.is_cancelled() => {
                let dto = found_dto(&lookup, &config.home);
                *slot = Slot::Ready(lookup);
                GradingLinkLookup::Found(dto)
            }
            Ok(lookup) => {
                drop(slot);
                skill_link::remove_all(&lookup.unpacked.root);
                GradingLinkLookup::Cancelled
            }
            Err(answer) => {
                if ours {
                    *slot = Slot::Idle;
                }
                answer
            }
        }
    }

    /// Cancel the look-up going now. False when none is going.
    pub fn cancel(&self) -> bool {
        match &*lock(&self.inner.slot) {
            Slot::Looking { cancel, .. } => {
                cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Drop the preview `lookup_id` and its staging folder. False when it is
    /// not the one kept.
    pub fn discard(&self, lookup_id: &str) -> bool {
        let mut slot = lock(&self.inner.slot);
        if !matches!(&*slot, Slot::Ready(lookup) if lookup.id == lookup_id) {
            return false;
        }
        let dropped = std::mem::replace(&mut *slot, Slot::Idle);
        drop(slot);
        drop_slot(dropped);
        true
    }

    /// Keep the skills at `paths` of the preview `lookup_id` (see the module
    /// docs).
    pub fn keep(&self, lookup_id: &str, paths: &[String]) -> GradingLinkKeep {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return GradingLinkKeep::NotConfigured;
        };
        if paths.is_empty() {
            return GradingLinkKeep::NothingChosen;
        }
        let lookup = {
            let mut slot = lock(&inner.slot);
            if !matches!(&*slot, Slot::Ready(lookup) if lookup.id == lookup_id) {
                return GradingLinkKeep::Expired;
            }
            let Slot::Ready(lookup) = std::mem::replace(
                &mut *slot,
                Slot::Keeping {
                    id: lookup_id.to_owned(),
                },
            ) else {
                return GradingLinkKeep::Expired;
            };
            lookup
        };
        // Put an unkept preview back, unless another look-up began meanwhile.
        let give_back = |lookup: Lookup| {
            let mut slot = lock(&inner.slot);
            if matches!(&*slot, Slot::Keeping { id } if id == lookup_id) {
                *slot = Slot::Ready(lookup);
            } else {
                drop(slot);
                skill_link::remove_all(&lookup.unpacked.root);
            }
        };
        let states = skill_link::keep_states(&lookup.found.skills, &config.home);
        let chosen: Vec<usize> = lookup
            .found
            .skills
            .iter()
            .enumerate()
            .filter(|(_, skill)| paths.contains(&skill.path))
            .map(|(at, _)| at)
            .collect();
        let known = paths
            .iter()
            .all(|path| lookup.found.skills.iter().any(|skill| &skill.path == path));
        if !known || chosen.iter().any(|at| states[*at] != KeepState::Keepable) {
            let dto = found_dto(&lookup, &config.home);
            give_back(lookup);
            return GradingLinkKeep::Changed(dto);
        }
        let Ok(agents) = skill_link::agents_folder(&config.home) else {
            give_back(lookup);
            return GradingLinkKeep::AgentsFolderUnusable;
        };
        let now = (config.clock)();
        let file = LinkedSkills::new(config.records.clone());
        let mut records = Vec::new();
        let mut kept_any = false;
        let mut skills = Vec::with_capacity(chosen.len());
        for (n, at) in chosen.into_iter().enumerate() {
            let skill = &lookup.found.skills[at];
            let kept = skill_link::keep(
                &lookup.unpacked,
                skill,
                &config.home,
                &agents,
                &format!("{}-{n}", lookup.id),
            );
            if let Kept::Kept { folder, claude } = &kept {
                kept_any = true;
                records.push(LinkedSkill {
                    folder: folder.to_string_lossy().into_owned(),
                    claude_link: match claude {
                        ClaudeLink::Linked(link) => Some(link.to_string_lossy().into_owned()),
                        _ => None,
                    },
                    repository: format!("{}/{}", lookup.link.owner, lookup.link.repo),
                    path: skill.path.clone(),
                    reference: lookup.link.reference.clone(),
                    commit: lookup.commit.clone(),
                    fetched_at_ms: lookup.fetched_at_ms,
                    kept_at_ms: now,
                });
            }
            skills.push(GradingLinkKept {
                path: skill.path.clone(),
                name: skill.name.clone(),
                result: kept_dto(kept, &config.home),
            });
        }
        // Each kept skill's source, in one change of the records file.
        let recorded = kept_any
            && file
                .update(|kept| {
                    records.into_iter().fold(true, |all, record| {
                        skill_link::recorded(kept, record) && all
                    })
                })
                .unwrap_or(false);
        skill_link::remove_all(&lookup.unpacked.root);
        {
            let mut slot = lock(&inner.slot);
            if matches!(&*slot, Slot::Keeping { id } if id == lookup_id) {
                *slot = Slot::Idle;
            }
        }
        if kept_any {
            (inner.changed)();
        }
        GradingLinkKeep::Done { skills, recorded }
    }

    /// Quitting: cancel a look-up going, drop a preview and its staging
    /// folder; nothing starts afterwards.
    pub fn shutdown(&self) {
        self.inner
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let dropped = std::mem::replace(&mut *lock(&self.inner.slot), Slot::Idle);
        drop_slot(dropped);
    }
}

/// End what a slot held: cancel a look-up going (it removes its own folder),
/// remove a preview's staging folder.
fn drop_slot(slot: Slot) {
    match slot {
        Slot::Looking { cancel, .. } => cancel.cancel(),
        Slot::Ready(lookup) => skill_link::remove_all(&lookup.unpacked.root),
        Slot::Idle | Slot::Keeping { .. } => {}
    }
}

/// Why a request did not give what was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refused {
    NotFound,
    RateLimited,
    TooLarge,
    TimedOut,
    Network,
}

impl From<Refused> for GradingLinkLookup {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::NotFound => Self::NotFound,
            Refused::RateLimited => Self::RateLimited,
            Refused::TooLarge => Self::TooLarge {
                limit: GradingLinkLimit::Download,
            },
            Refused::TimedOut => Self::TimedOut,
            Refused::Network => Self::NetworkFailed,
        }
    }
}

/// The HTTP client of one look-up: GitHub without signing in, the system's
/// proxy unless turned off, redirects only on the same host.
fn client(config: &LinkConfig) -> Option<reqwest::Client> {
    // The same provider the updater installs; reqwest is built without one.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let redirects = reqwest::redirect::Policy::custom(|attempt| {
        let same_host = attempt.previous().first().is_some_and(|first| {
            first.scheme() == attempt.url().scheme()
                && first.host_str() == attempt.url().host_str()
                && first.port_or_known_default() == attempt.url().port_or_known_default()
        });
        if attempt.previous().len() > MAX_REDIRECTS || !same_host {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(config.limits.connect)
        .redirect(redirects);
    let builder = if config.system_proxy {
        builder
    } else {
        builder.no_proxy()
    };
    builder.build().ok()
}

/// What a non-success status means.
fn refused(response: &reqwest::Response) -> Refused {
    let status = response.status().as_u16();
    let exhausted = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        == Some("0");
    match status {
        429 => Refused::RateLimited,
        403 if exhausted => Refused::RateLimited,
        401 | 403 | 404 | 410 | 422 | 451 => Refused::NotFound,
        _ => Refused::Network,
    }
}

fn request_error(error: &reqwest::Error) -> Refused {
    if error.is_timeout() {
        Refused::TimedOut
    } else {
        Refused::Network
    }
}

/// The body of a successful answer, at most `cap` bytes.
async fn body(mut response: reqwest::Response, cap: u64) -> Result<Vec<u8>, Refused> {
    if !response.status().is_success() {
        return Err(refused(&response));
    }
    if response.content_length().is_some_and(|length| length > cap) {
        return Err(Refused::TooLarge);
    }
    let mut out = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| request_error(&error))?
    {
        if out.len() as u64 + chunk.len() as u64 > cap {
            return Err(Refused::TooLarge);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// The commit the link's ref (or the default branch) names now.
async fn resolve_commit(
    client: &reqwest::Client,
    config: &LinkConfig,
    link: &GithubLink,
) -> Result<String, Refused> {
    let url = format!(
        "{}/repos/{}/{}/commits/{}",
        config.endpoints.api,
        link.owner,
        link.repo,
        link.reference.as_deref().unwrap_or("HEAD")
    );
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github.sha")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(config.limits.commit)
        .send()
        .await
        .map_err(|error| request_error(&error))?;
    let answer = body(response, MAX_COMMIT_ANSWER)
        .await
        .map_err(|refused| match refused {
            Refused::TooLarge => Refused::Network,
            other => other,
        })?;
    let commit = String::from_utf8(answer)
        .map(|text| text.trim().to_owned())
        .map_err(|_| Refused::Network)?;
    if skill_link::is_commit(&commit) {
        Ok(commit)
    } else {
        Err(Refused::Network)
    }
}

/// That commit's archive.
async fn download(
    client: &reqwest::Client,
    config: &LinkConfig,
    link: &GithubLink,
    commit: &str,
) -> Result<Vec<u8>, Refused> {
    let url = format!(
        "{}/{}/{}/tar.gz/{commit}",
        config.endpoints.codeload, link.owner, link.repo
    );
    let response = client
        .get(url)
        .timeout(config.limits.archive)
        .send()
        .await
        .map_err(|error| request_error(&error))?;
    body(response, config.limits.download_bytes).await
}

/// Resolve, download, unpack and find; the unpacked folder is removed on
/// any failure.
async fn fetch(
    config: &LinkConfig,
    id: &str,
    link: &GithubLink,
    cancel: &Arc<Cancel>,
) -> Result<Lookup, GradingLinkLookup> {
    let client = client(config).ok_or(GradingLinkLookup::Failed)?;
    let commit = race(resolve_commit(&client, config, link), cancel)
        .await
        .ok_or(GradingLinkLookup::Cancelled)??;
    let fetched_at_ms = (config.clock)();
    let archive = race(download(&client, config, link, &commit), cancel)
        .await
        .ok_or(GradingLinkLookup::Cancelled)??;
    let folder = staging_folder(&config.staging, id).ok_or(GradingLinkLookup::Failed)?;
    let flag = cancel.flag.clone();
    let limits = config.limits.archive_limits;
    let (prefix, repo) = (link.path.clone(), link.repo.clone());
    let into = folder.clone();
    let unpacked = tauri::async_runtime::spawn_blocking(move || {
        let unpacked = skill_link::unpack(archive.as_slice(), &into, &prefix, limits, &flag)
            .map_err(archive_answer)?;
        let found = skill_link::find(&unpacked, &prefix, &repo).map_err(|error| match error {
            FindError::PathNotFound => Unfound::PathNotFound,
            FindError::NoSkill => Unfound::NoSkill,
        })?;
        Ok::<_, Unfound>((unpacked, found))
    })
    .await;
    match unpacked {
        Ok(Ok((unpacked, found))) => Ok(Lookup {
            id: id.to_owned(),
            link: link.clone(),
            commit,
            fetched_at_ms,
            unpacked,
            found,
        }),
        Ok(Err(unfound)) => {
            skill_link::remove_all(&folder);
            Err(unfound.into())
        }
        Err(_) => {
            skill_link::remove_all(&folder);
            Err(GradingLinkLookup::Failed)
        }
    }
}

/// Why an unpacked archive gave no preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unfound {
    Archive(ArchiveError),
    PathNotFound,
    NoSkill,
}

fn archive_answer(error: ArchiveError) -> Unfound {
    Unfound::Archive(error)
}

impl From<Unfound> for GradingLinkLookup {
    fn from(unfound: Unfound) -> Self {
        match unfound {
            Unfound::Archive(ArchiveError::TooManyEntries) => Self::TooLarge {
                limit: GradingLinkLimit::Entries,
            },
            Unfound::Archive(ArchiveError::TooLarge) => Self::TooLarge {
                limit: GradingLinkLimit::Unpacked,
            },
            Unfound::Archive(ArchiveError::Unsafe) => Self::UnsafeArchive,
            Unfound::Archive(ArchiveError::Malformed) => Self::BadArchive,
            Unfound::Archive(ArchiveError::PathNotFound) | Unfound::PathNotFound => {
                Self::PathNotFound
            }
            Unfound::Archive(ArchiveError::Cancelled) => Self::Cancelled,
            Unfound::Archive(ArchiveError::Io) => Self::Failed,
            Unfound::NoSkill => Self::NoSkill,
        }
    }
}

/// `<staging>/<id>`, new and private, inside a private `staging`.
#[cfg(unix)]
fn staging_folder(staging: &Path, id: &str) -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let made = |folder: &Path| match std::fs::DirBuilder::new().mode(0o700).create(folder) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::AlreadyExists,
    };
    if let Some(parent) = staging.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    if !made(staging) {
        return None;
    }
    let private = std::fs::symlink_metadata(staging)
        .is_ok_and(|meta| meta.is_dir() && meta.permissions().mode() & 0o077 == 0);
    if !private {
        return None;
    }
    let folder = staging.join(id);
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&folder)
        .ok()
        .map(|()| folder)
}

#[cfg(not(unix))]
fn staging_folder(_: &Path, _: &str) -> Option<PathBuf> {
    None
}

/// `path` under `home` as `~/…`; as it is otherwise.
pub fn shown(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.to_string_lossy()),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// A look-up's preview, with whether each skill can be kept now.
fn found_dto(lookup: &Lookup, home: &Path) -> GradingLinkFound {
    preview(
        &lookup.id,
        &lookup.link,
        &lookup.commit,
        &lookup.found,
        home,
    )
}

/// [`found_dto`] over its parts.
pub fn preview(
    id: &str,
    link: &GithubLink,
    commit: &str,
    found: &Found,
    home: &Path,
) -> GradingLinkFound {
    let states = skill_link::keep_states(&found.skills, home);
    let skills: Vec<GradingLinkSkill> = found
        .skills
        .iter()
        .zip(states)
        .map(|(skill, state)| GradingLinkSkill {
            path: skill.path.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            files: skill.files,
            bytes: skill.bytes,
            rubric: match &skill.rubric {
                RubricStatus::Ready(rubric) => GradingInstalledRubric::Ready {
                    id: rubric.id.clone(),
                },
                RubricStatus::Missing => GradingInstalledRubric::Missing,
                RubricStatus::Problem(reason) => GradingInstalledRubric::Problem {
                    reason: (*reason).into(),
                },
            },
            folder_name: skill.folder_name.clone(),
            keep: match state {
                KeepState::Keepable => GradingLinkKeepState::Keepable,
                KeepState::NameInAgents(folder) => GradingLinkKeepState::NameInAgents {
                    folder: shown(home, &folder),
                },
                KeepState::NameInClaude(folder) => GradingLinkKeepState::NameInClaude {
                    folder: shown(home, &folder),
                },
                KeepState::SameNameAbove(path) => GradingLinkKeepState::SameNameAbove { path },
                KeepState::UnusableName => GradingLinkKeepState::UnusableName,
                KeepState::Problem(SkillProblem::TooLarge) => GradingLinkKeepState::TooLarge,
                KeepState::Problem(SkillProblem::LinkLeavesFolder) => {
                    GradingLinkKeepState::LinkLeavesFolder
                }
                KeepState::Problem(SkillProblem::InstructionsUnreadable) => {
                    GradingLinkKeepState::InstructionsUnreadable
                }
            },
        })
        .collect();
    let keepable = skills
        .iter()
        .filter(|skill| skill.keep == GradingLinkKeepState::Keepable)
        .count();
    GradingLinkFound {
        lookup_id: id.to_owned(),
        source: GradingLinkSource {
            repository: link.repository(),
            path: link.path.clone(),
            reference: link.reference.clone(),
            commit: commit.to_owned(),
            short_commit: skill_link::short_commit(commit).to_owned(),
        },
        skills,
        keepable: u32::try_from(keepable).unwrap_or(u32::MAX),
        incomplete: found.incomplete,
    }
}

/// What keeping one skill did, as shown.
pub fn kept_dto(kept: Kept, home: &Path) -> GradingLinkKeptResult {
    match kept {
        Kept::Kept { folder, claude } => GradingLinkKeptResult::Kept {
            folder: shown(home, &folder),
            claude: match claude {
                ClaudeLink::Linked(link) => GradingLinkClaude::Linked {
                    link: shown(home, &link),
                },
                ClaudeLink::NameTaken => GradingLinkClaude::NameTaken,
                ClaudeLink::NoClaudeFolder => GradingLinkClaude::NoClaudeFolder,
                ClaudeLink::Failed => GradingLinkClaude::Failed,
            },
        },
        Kept::NameTaken => GradingLinkKeptResult::NameTaken,
        Kept::Failed => GradingLinkKeptResult::Failed,
    }
}

#[cfg(test)]
#[path = "skill_link_tests.rs"]
mod tests;
