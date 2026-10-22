//! Grading runs in the app: one grading skill on one project, on the
//! user's request, saved in the database.
//!
//! * **Skills.** The built-in skills bundled with the app, then the skills
//!   the user added from the skills they installed ([`GradingService::installed`]
//!   lists those, [`GradingService::add_skill`] adds one,
//!   [`GradingService::remove_skill`] takes one off Scores, never while it
//!   is grading). An added skill is found again each time it is read or run
//!   ([`installed::locate`]: a plugin's skill in the plugin's current
//!   install) and opened from there; when its folder, plugin or rubric has
//!   gone or is no longer valid, it keeps its card and stored grades and
//!   says why ([`GradingSkillProblem`]), and nothing starts. A run copies an
//!   added skill within [`CopyLimits::INSTALLED`], and fails as
//!   `skill_changed` when the copy's rubric version is not the one stored
//!   with the run at its start.
//!
//! * **Rubrics.** "Make a rubric" ([`GradingService::make_rubric`]) runs
//!   Claude Code with the bundled `xtrace-rubric-maker` helper on a staged
//!   copy of an installed skill the "Add a skill" rule offers it for (one
//!   that needs a rubric, or whose rubric has a problem, and that Scores
//!   does not already have under another skill), contained like a grading
//!   ([`xt_judge::maker`]); one retry gets the rubric reader's reason. The
//!   helper may instead refuse with a short reason when the skill cannot be
//!   used to score projects: nothing is saved, nothing is retried, and the
//!   job ends `refused` with that reason. Only XTrace writes the checked
//!   rubric ([`save_made`]), with its header `version` set by XTrace
//!   ([`xt_judge::maker::next_version`]), and only while the job is not cancelled (checked
//!   under the slot's lock): next to the skill's `SKILL.md` for a skill
//!   folder, never over an existing file (a remade one moves the old file to
//!   `xtrace-rubric.md.bak`, and back when the new one cannot be put in
//!   place); in XTrace's rubric store
//!   (`<data>/rubrics/<key>/`, [`xt_judge::rubric_store`]) for a Claude Code
//!   plugin's skill or a folder that cannot be written. Which rubric applies
//!   to a skill is decided by one rule
//!   ([`xt_judge::rubric_store::applicable`]) for discovery, opening an
//!   added skill and staging a run alike. A made rubric whose id another
//!   skill has gets the smallest free `-2` … `-99` suffix ([`settle_id`]);
//!   remaking an added skill's rubric keeps the id its card has. Jobs live in
//!   memory: the running one ([`GradingService::overview`] names it) and
//!   each location's latest outcome. A refusal is also kept across restarts
//!   in `<data>/rubric-refusals.json` ([`xt_judge::refusals`]) with the
//!   skill's fingerprint taken when its job started; it is listed only while
//!   the skill's fingerprint is unchanged, and making a rubric again drops
//!   it. `grading://rubric-progress` reports
//!   a job's elapsed time; `grading://changed` follows its start, a saved
//!   rubric, and its end.
//! * **One at a time.** A run or a rubric-making job holds the service's
//!   single slot from the moment it is accepted until it is finished; a
//!   second request answers `busy`.
//!   After [`GradingService::shutdown`] nothing starts. The slot's lock is
//!   only ever held briefly and never while the database is waited for, so
//!   Cancel (a synchronous command) never waits on storage.
//! * **Only a listed project.** The folder must be one a fresh project list
//!   read names, given as exactly its resolved path (no link, `..` or
//!   trailing slash: the path must equal what the file system resolves it
//!   to, and be a real folder). The check is made when the run is requested
//!   and again right before Claude Code launches.
//! * **Saved first.** The `running` row is committed before anything runs;
//!   it is finished exactly once, into `succeeded`, `failed`, `cancelled` or
//!   `interrupted`. `grading://changed` is published after every committed
//!   status change, never before.
//! * **Progress.** `grading://progress` carries the run ID, `running` or
//!   `finished` and the whole seconds elapsed: when the run starts, about
//!   every five seconds while it runs, and once at the end.
//! * **Where it runs.** In a private staging folder without whitespace
//!   under the system temporary folder ([`xt_judge::staging`]); only the
//!   result, the report and the cards beside it are kept, in
//!   `<data>/grades/<run id>/`, and the staging folder is removed. The kept
//!   folders of the newest [`KEEP_FOLDERS`] succeeded runs stay; older ones
//!   are removed when a run finishes. A content purge removes every kept
//!   folder and the kept rubric-making refusals
//!   ([`GradingService::purge_with_folders`]); rows stay.
//! * **Quitting.** [`GradingService::shutdown`] cancels the run, whose process
//!   group gets `SIGTERM` and then `SIGKILL` after the runner's grace; the
//!   run is saved `interrupted` (`app_closed`) and its staging folder
//!   removed, within [`SHUTDOWN_BOUND`]. At the next start,
//!   [`GradingService::recover`] marks whatever is still `running`
//!   interrupted and removes leftover staging folders of this database's
//!   runs. Each run records its Claude Code process (ID and start time) in
//!   its staging folder when it launches; a process that outlived a crash
//!   of the app is stopped there first (`SIGTERM`, then `SIGKILL` after the
//!   runner's grace), only when that ID still names exactly the recorded
//!   process and it still leads its group
//!   ([`xt_judge::staging::stop_recorded_leader`]).
//!
//! Fixture mode's service is not configured: it answers `not_configured`,
//! never runs anything and never reads a skill.
use crate::{
    grading_dto::{
        GradingFailure, GradingInsight, GradingInstalledRubric, GradingInstalledScores,
        GradingInstalledSkills, GradingOverview, GradingProgress, GradingProgressState,
        GradingRubricCriterion, GradingRubricFailure, GradingRubricJob, GradingRubricMake,
        GradingRubricNow, GradingRubricProgress, GradingRun, GradingRuns, GradingSkillAdd,
        GradingSkillInfo, GradingSkillOrigin, GradingSkillProblem, GradingSkillRemove,
        GradingSkills, GradingStart,
    },
    projects_dto::ProjectList,
    state::StateError,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use xt_judge::{
    CancelFlag, CopyLimits, GradeError, GradeResult, GradeRun, GradingSkill, Limits, Rubric,
    RubricError, Staging,
    installed::{self, Location, RubricStatus},
    maker::{self, MakeError, Written},
    refusals::{self, Refusal, RefusalStore},
    rubric,
    rubric_store::{self, RubricStore, SaveError},
    runner,
    skill::{self, BUILTIN},
    staging::{private_parent, remove_run_folders, stop_recorded_leader},
};
use xt_store::{
    Store,
    added_skills::AddedSkill,
    grades::{GradeFinish, GradeRow, NewGradeRun},
};

/// Where the bundled skills live below the app's resource folder.
pub const BUNDLE_RESOURCE: &str = "grading-skills";
/// The permanent grade folders' parent inside the app's data folder.
pub const GRADES_DIR: &str = "grades";
/// How many succeeded runs keep their grade folder. Settings states this
/// number in words (`retention-grades` in `SettingsPage.tsx`).
pub const KEEP_FOLDERS: u32 = 50;
/// Most runs one `grading_runs` returns.
pub const MAX_RUNS: u32 = 100;
/// How often a running run reports progress.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(5);
/// How long quitting waits for a cancelled run to be saved and cleaned up:
/// the runner's `SIGTERM` grace ([`runner::DEFAULT_TERM_GRACE`], 3 s), the
/// time a terminated group's pipes are drained
/// ([`xt_probes::process::DRAIN_GRACE`], 5 s), and a margin to save the row
/// and remove the staging folder. The app waits for it alongside the other
/// services' shutdown ([`GradingService::shutdown_in_background`]).
pub const SHUTDOWN_BOUND: Duration = Duration::from_secs(12);

/// Called after every committed status change.
pub type Publish = Arc<dyn Fn() + Send + Sync>;
/// Called with each progress report.
pub type PublishProgress = Arc<dyn Fn(&GradingProgress) + Send + Sync>;
/// Called with each rubric-making progress report.
pub type PublishRubricProgress = Arc<dyn Fn(&GradingRubricProgress) + Send + Sync>;
/// How many locations' latest finished rubric-making jobs are remembered.
pub const MAX_REMEMBERED_JOBS: usize = 64;
/// The mode of a rubric saved in a skill's own folder: readable like the
/// skill's other files.
const FOLDER_RUBRIC_MODE: u32 = 0o644;
/// The mode of a rubric saved in XTrace's rubric store.
const STORE_RUBRIC_MODE: u32 = 0o600;
/// A fresh project list read.
pub type ReadProjects = Arc<dyn Fn() -> ProjectList + Send + Sync>;
/// The wall clock, in Unix milliseconds.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The database the service saves runs in: the app's state, reached when
/// needed and refused once it is closed.
pub trait GradeStore: Send + Sync {
    fn use_store(
        &self,
        call: &mut dyn FnMut(&mut Store) -> xt_store::Result<()>,
    ) -> Result<(), StateError>;
}

impl GradeStore for crate::state::AppState {
    fn use_store(
        &self,
        call: &mut dyn FnMut(&mut Store) -> xt_store::Result<()>,
    ) -> Result<(), StateError> {
        crate::state::AppState::with_store(self, |store| Ok(call(store)?))
    }
}

impl<T: GradeStore> GradeStore for Arc<T> {
    fn use_store(
        &self,
        call: &mut dyn FnMut(&mut Store) -> xt_store::Result<()>,
    ) -> Result<(), StateError> {
        (**self).use_store(call)
    }
}

/// The app's managed state through its handle, so the service can be built
/// before the state is managed.
pub struct ManagedState<R: tauri::Runtime>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> GradeStore for ManagedState<R> {
    fn use_store(
        &self,
        call: &mut dyn FnMut(&mut Store) -> xt_store::Result<()>,
    ) -> Result<(), StateError> {
        use tauri::Manager;
        self.0
            .try_state::<crate::state::AppState>()
            .ok_or(StateError::Closed)?
            .with_store(|store| Ok(call(store)?))
    }
}

fn store_call<T>(
    store: &dyn GradeStore,
    call: impl FnOnce(&mut Store) -> xt_store::Result<T>,
) -> Result<T, StateError> {
    let mut call = Some(call);
    let mut out = None;
    store.use_store(&mut |store| {
        if let Some(call) = call.take() {
            out = Some(call(store)?);
        }
        Ok(())
    })?;
    out.ok_or(StateError::Poisoned)
}

/// Where a run's environment comes from.
#[derive(Clone)]
pub enum RunEnvironment {
    /// This process's environment, read when a run is requested.
    Process,
    /// A fixed one (tests).
    Fixed(Vec<(OsString, OsString)>),
}

impl RunEnvironment {
    fn read(&self) -> Vec<(OsString, OsString)> {
        match self {
            Self::Process => runner::inherited_env(),
            Self::Fixed(env) => env.clone(),
        }
    }
}

/// Everything a configured service needs.
pub struct GradingConfig {
    /// The folder holding `<skill id>/skill` for each built-in skill.
    pub skills_root: PathBuf,
    /// The native home whose installed skills are listed
    /// ([`xt_judge::installed`]).
    pub home: PathBuf,
    /// The bundled rubric maker's folder ([`xt_judge::maker::Helper`]).
    pub maker_dir: PathBuf,
    /// `<data>/rubrics`: the rubrics XTrace keeps ([`RubricStore`]).
    pub rubric_store: PathBuf,
    /// `<data>/rubric-refusals.json`: the rubric maker's refusals
    /// ([`RefusalStore`]).
    pub refusals: PathBuf,
    /// Names this app's rubric-making staging folders ([`maker::data_key`]).
    pub maker_key: String,
    /// The bounds of one rubric-making run.
    pub maker_limits: Limits,
    /// The system temporary folder; staging goes in `xtrace-grading/` there.
    pub temp_dir: PathBuf,
    /// `<data>/grades`.
    pub grades_dir: PathBuf,
    pub environment: RunEnvironment,
    pub projects: ReadProjects,
    pub limits: Limits,
    pub keep_folders: u32,
    pub progress_every: Duration,
    pub shutdown_bound: Duration,
    /// Stamps a run's start and finish (tests move it).
    pub clock: Clock,
    /// Called on the run's thread right before the skill is copied (tests
    /// change the skill there).
    #[cfg(test)]
    pub before_staging: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Called on a rubric-making job's thread right before its rubric is
    /// saved (tests cancel it there).
    #[cfg(test)]
    pub before_save: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl GradingConfig {
    /// The production settings over the given folders.
    pub fn new(
        skills_root: PathBuf,
        home: PathBuf,
        data_dir: &Path,
        projects: ReadProjects,
    ) -> Self {
        Self {
            maker_dir: skills_root.join(maker::HELPER_ID),
            rubric_store: data_dir.join(rubric_store::STORE_DIR),
            refusals: data_dir.join(refusals::REFUSALS_FILE),
            maker_key: maker::data_key(data_dir),
            maker_limits: Limits {
                deadline: maker::DEFAULT_DEADLINE,
                ..Limits::default()
            },
            skills_root,
            home,
            temp_dir: std::env::temp_dir(),
            grades_dir: data_dir.join(GRADES_DIR),
            environment: RunEnvironment::Process,
            projects,
            limits: Limits::default(),
            keep_folders: KEEP_FOLDERS,
            progress_every: PROGRESS_EVERY,
            shutdown_bound: SHUTDOWN_BOUND,
            clock: Arc::new(now_ms),
            #[cfg(test)]
            before_staging: None,
            #[cfg(test)]
            before_save: None,
        }
    }
}

struct Active {
    /// The run's ID, or a rubric-making job's.
    run_id: String,
    /// The skill a run grades with: an added skill is not removed meanwhile.
    /// None for a rubric-making job.
    skill_id: Option<String>,
    cancel: CancelFlag,
}

/// The rubric-making jobs, in memory only.
#[derive(Default)]
struct RubricJobs {
    /// The job going now.
    running: Option<GradingRubricNow>,
    /// Each location's latest finished job but a refusal (those are kept
    /// in [`Inner::refusals`]), oldest first, at most
    /// [`MAX_REMEMBERED_JOBS`].
    finished: Vec<(String, GradingRubricJob)>,
}

impl RubricJobs {
    /// Each location's latest job: the running one, else its last outcome.
    fn by_location(&self) -> HashMap<String, GradingRubricJob> {
        let mut jobs: HashMap<String, GradingRubricJob> = self.finished.iter().cloned().collect();
        if let Some(running) = &self.running {
            jobs.insert(
                running.location.clone(),
                GradingRubricJob::Running {
                    job_id: running.job_id.clone(),
                    started_at_ms: running.started_at_ms,
                },
            );
        }
        jobs
    }

    /// The running job of `job_id` ended as `outcome`. A refusal replaces
    /// the location's last outcome but is kept with the refusals, not here.
    fn finish(&mut self, job_id: &str, location: &str, outcome: GradingRubricJob) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.job_id == job_id)
        {
            self.running = None;
        }
        self.finished.retain(|(seen, _)| seen != location);
        if matches!(outcome, GradingRubricJob::Refused { .. }) {
            return;
        }
        self.finished.push((location.to_owned(), outcome));
        if self.finished.len() > MAX_REMEMBERED_JOBS {
            self.finished.remove(0);
        }
    }
}

struct Inner {
    config: Option<GradingConfig>,
    store: Arc<dyn GradeStore>,
    changed: Publish,
    progress: PublishProgress,
    rubric_progress: PublishRubricProgress,
    /// The rubric-making jobs. Held only briefly, never across a database
    /// call; taken after `active` when both are held.
    jobs: Mutex<RubricJobs>,
    /// The kept refusals, read from their file at first use (`None` until
    /// then) and written back after each change. Held across that small
    /// file's read and write only, never with `active` or `jobs`.
    refusals: Mutex<Option<Vec<Refusal>>>,
    /// The one run that may be going; `None` while idle. Held only briefly:
    /// never across a database call, so [`GradingService::cancel`] never
    /// waits on storage.
    active: Mutex<Option<Active>>,
    /// Held by a removal across its check of the slot and its delete, and
    /// by a start of an added skill across its re-check of the stored skill
    /// and its claim of the slot: a skill is never removed while it grades,
    /// and a removed skill never starts.
    membership: Mutex<()>,
    idle: Condvar,
    stopped: AtomicBool,
    /// Held while kept grade folders are written and their row committed,
    /// and while a purge clears content and removes those folders, so the
    /// two never interleave.
    folders: Mutex<()>,
}

pub struct GradingService {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// Integers JavaScript reads exactly.
const MAX_EXACT: i64 = (1 << 53) - 1;

impl GradingService {
    /// A configured service. Nothing is read or started until asked.
    pub fn new(
        config: GradingConfig,
        store: Arc<dyn GradeStore>,
        changed: Publish,
        progress: PublishProgress,
        rubric_progress: PublishRubricProgress,
    ) -> Self {
        Self::with(Some(config), store, changed, progress, rubric_progress)
    }

    /// Fixture mode: every command answers `not_configured`.
    pub fn not_configured() -> Self {
        struct Closed;
        impl GradeStore for Closed {
            fn use_store(
                &self,
                _: &mut dyn FnMut(&mut Store) -> xt_store::Result<()>,
            ) -> Result<(), StateError> {
                Err(StateError::Closed)
            }
        }
        Self::with(
            None,
            Arc::new(Closed),
            Arc::new(|| {}),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        )
    }

    fn with(
        config: Option<GradingConfig>,
        store: Arc<dyn GradeStore>,
        changed: Publish,
        progress: PublishProgress,
        rubric_progress: PublishRubricProgress,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                store,
                changed,
                progress,
                rubric_progress,
                jobs: Mutex::new(RubricJobs::default()),
                refusals: Mutex::new(None),
                active: Mutex::new(None),
                membership: Mutex::new(()),
                idle: Condvar::new(),
                stopped: AtomicBool::new(false),
                folders: Mutex::new(()),
            }),
        }
    }

    /// The built-in skills, read from their bundled rubric's header, then
    /// the added skills in the order they were added, each read from its
    /// own folder now (or, when it cannot be opened, as stored, with why).
    pub fn skills(&self) -> GradingSkills {
        let Some(config) = &self.inner.config else {
            return GradingSkills::NotConfigured;
        };
        let Some(mut skills) = BUILTIN
            .iter()
            .map(|id| {
                open_bundled(config, id).map(|skill| info(&skill, GradingSkillOrigin::BuiltIn))
            })
            .collect::<Option<Vec<_>>>()
        else {
            return GradingSkills::Failed;
        };
        let Ok(added) = store_call(&*self.inner.store, |store| store.added_skills()) else {
            return GradingSkills::Failed;
        };
        skills.extend(added.iter().map(|skill| added_info(config, skill)));
        GradingSkills::Loaded { skills }
    }

    /// The skills installed under the native home, for the "Add a skill"
    /// dialog ([`crate::grading_picker::picker`]). Reading this starts and
    /// writes nothing.
    pub fn installed(&self) -> GradingInstalledSkills {
        let Some(config) = &self.inner.config else {
            return GradingInstalledSkills::NotConfigured;
        };
        let Ok(added) = store_call(&*self.inner.store, |store| store.added_skills()) else {
            return GradingInstalledSkills::Failed;
        };
        let mut jobs = lock(&self.inner.jobs).by_location();
        let found = discover(config);
        let kept = kept_refusals(&self.inner, config, |kept| kept.clone());
        for refusal in kept {
            if jobs.contains_key(&refusal.location) {
                continue;
            }
            let Some(skill) = found
                .skills
                .iter()
                .find(|skill| skill.location.text().as_deref() == Some(&refusal.location))
            else {
                continue;
            };
            if refusals::in_force(&refusal, refusals::fingerprint(&skill.folder).as_deref()) {
                jobs.insert(
                    refusal.location,
                    GradingRubricJob::Refused {
                        job_id: refusal.job_id,
                        reason: refusal.reason,
                    },
                );
            }
        }
        GradingInstalledSkills::Loaded(crate::grading_picker::picker_with(
            found, BUILTIN, &added, &jobs,
        ))
    }

    /// Add the installed skill at `location` to Scores. The location must be
    /// exactly one a fresh discovery marks addable
    /// ([`crate::grading_picker::picker`]), and its folder is opened again
    /// before it is stored. `grading://changed` follows a stored addition.
    pub fn add_skill(&self, location: &str) -> GradingSkillAdd {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return GradingSkillAdd::NotConfigured;
        };
        let Ok(added) = store_call(&*inner.store, |store| store.added_skills()) else {
            return GradingSkillAdd::Failed;
        };
        let found = crate::grading_picker::picker(discover(config), BUILTIN, &added);
        let Some(row) = found.skills.iter().find(|row| row.location == location) else {
            return GradingSkillAdd::NotFound;
        };
        match row.scores {
            GradingInstalledScores::Addable => {}
            GradingInstalledScores::NotReady => return GradingSkillAdd::NotReady,
            GradingInstalledScores::IdClash { .. } => return GradingSkillAdd::IdClash,
            GradingInstalledScores::OnScores
            | GradingInstalledScores::BuiltIn
            | GradingInstalledScores::IdOnScores { .. } => {
                return GradingSkillAdd::AlreadyOnScores;
            }
        }
        // Opened again: it must still carry the id the rule just decided on.
        let listed = match &row.rubric {
            crate::grading_dto::GradingInstalledRubric::Ready { id } => id.as_str(),
            _ => return GradingSkillAdd::NotReady,
        };
        let Some(location) = Location::parse(location) else {
            return GradingSkillAdd::NotFound;
        };
        let Ok(skill) = open_located(config, Path::new(&row.folder), &location, listed) else {
            return GradingSkillAdd::NotReady;
        };
        let skill_id = skill.id().to_owned();
        let stored = store_call(&*inner.store, |store| {
            store.added_skill_insert(&AddedSkill {
                skill_id: skill_id.clone(),
                location: row.location.clone(),
                name: skill.name().to_owned(),
                description: skill.description().to_owned(),
                rubric_version: skill.version(),
                added_at_ms: inner.now().max(0),
            })
        });
        match stored {
            Ok(true) => {
                (inner.changed)();
                GradingSkillAdd::Added { skill_id }
            }
            // Added meanwhile by another request.
            Ok(false) => GradingSkillAdd::AlreadyOnScores,
            Err(_) => GradingSkillAdd::Failed,
        }
    }

    /// Take the added skill `skill_id` off Scores. Its stored grades and its
    /// files stay. Built-in skills cannot be removed, and neither can a
    /// skill while it grades (the slot is held meanwhile, so no run of it
    /// starts either). `grading://changed` follows a removal.
    pub fn remove_skill(&self, skill_id: &str) -> GradingSkillRemove {
        let inner = &self.inner;
        if inner.config.is_none() {
            return GradingSkillRemove::NotConfigured;
        }
        if BUILTIN.contains(&skill_id) {
            return GradingSkillRemove::BuiltIn;
        }
        // No start of this skill claims the slot meanwhile (see `membership`);
        // the slot itself is only looked at.
        let _membership = lock(&inner.membership);
        if lock(&inner.active)
            .as_ref()
            .is_some_and(|active| active.skill_id.as_deref() == Some(skill_id))
        {
            return GradingSkillRemove::Running;
        }
        let removed = store_call(&*inner.store, |store| store.added_skill_remove(skill_id));
        match removed {
            Ok(true) => {
                (inner.changed)();
                GradingSkillRemove::Removed
            }
            Ok(false) => GradingSkillRemove::NotFound,
            Err(_) => GradingSkillRemove::Failed,
        }
    }

    /// The newest runs first, at most [`MAX_RUNS`].
    pub fn runs(&self) -> GradingRuns {
        if self.inner.config.is_none() {
            return GradingRuns::NotConfigured;
        }
        let Ok(rows) = store_call(&*self.inner.store, |store| store.grades(MAX_RUNS)) else {
            return GradingRuns::Failed;
        };
        rows.into_iter()
            .map(run_dto)
            .collect::<Option<Vec<_>>>()
            .map_or(GradingRuns::Failed, |runs| GradingRuns::Loaded { runs })
    }

    /// The Scores page's answer ([`crate::grading_overview::overview`]) over
    /// a fresh project list read, the bundled skills and the stored runs,
    /// read in one snapshot. Reading this starts nothing.
    pub fn overview(&self) -> GradingOverview {
        let Some(config) = &self.inner.config else {
            return GradingOverview::NotConfigured;
        };
        let GradingSkills::Loaded { skills } = self.skills() else {
            return GradingOverview::Failed;
        };
        let ProjectList::Loaded(list) = (config.projects)() else {
            return GradingOverview::Failed;
        };
        let pairs = crate::grading_overview::pairs(&list.projects, &skills);
        let Ok(read) = store_call(&*self.inner.store, |store| store.grade_pairs(&pairs)) else {
            return GradingOverview::Failed;
        };
        let Some(mut loaded) = crate::grading_overview::overview(list.projects, &skills, &read)
        else {
            return GradingOverview::Failed;
        };
        loaded.making_rubric = lock(&self.inner.jobs).running.clone().map(Box::new);
        GradingOverview::Loaded(loaded)
    }

    /// Make a rubric for the installed skill at `location`: exactly one the
    /// "Add a skill" rule ([`crate::grading_picker::picker`]), re-run over a
    /// fresh discovery, offers a rubric action for. The job runs on its own
    /// thread in the one slot; this returns once it has started.
    /// `grading://changed` follows the start, a saved rubric and the end.
    pub fn make_rubric(&self, location: &str) -> GradingRubricMake {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return GradingRubricMake::NotConfigured;
        };
        if inner.stopped.load(Ordering::SeqCst) {
            return GradingRubricMake::Stopped;
        }
        let Ok(added) = store_call(&*inner.store, |store| store.added_skills()) else {
            return GradingRubricMake::Failed;
        };
        let found = discover(config);
        let listed = crate::grading_picker::picker(found.clone(), BUILTIN, &added);
        let Some(row) = listed.skills.iter().find(|row| row.location == location) else {
            return GradingRubricMake::NotFound;
        };
        if row.rubric_action.is_none() {
            return match row.rubric {
                GradingInstalledRubric::Ready { .. } => GradingRubricMake::AlreadyReady,
                _ => GradingRubricMake::NotOffered,
            };
        }
        let Some(skill) = found
            .skills
            .into_iter()
            .find(|skill| skill.location.text().as_deref() == Some(location))
        else {
            return GradingRubricMake::NotFound;
        };
        let existing = match &skill.rubric {
            RubricStatus::Ready(_) => return GradingRubricMake::AlreadyReady,
            RubricStatus::Missing => None,
            RubricStatus::Problem(problem) => Some(*problem),
        };
        if maker::Helper::open(&config.maker_dir).is_err() {
            return GradingRubricMake::HelperUnavailable;
        }
        let environment = config.environment.read();
        let Ok(claude) = runner::find_claude(&environment) else {
            return GradingRubricMake::ClaudeUnavailable;
        };
        let Some(job_id) = crate::telemetry::random_uuid() else {
            return GradingRubricMake::Failed;
        };
        let cancel = CancelFlag::new();
        let started_at_ms = inner.now();
        {
            let mut active = lock(&inner.active);
            if active.is_some() {
                return GradingRubricMake::Busy;
            }
            // Checked again under the slot, as a start of a run is.
            if inner.stopped.load(Ordering::SeqCst) {
                return GradingRubricMake::Stopped;
            }
            *active = Some(Active {
                run_id: job_id.clone(),
                skill_id: None,
                cancel: cancel.clone(),
            });
            lock(&inner.jobs).running = Some(GradingRubricNow {
                job_id: job_id.clone(),
                location: location.to_owned(),
                name: skill.name.clone(),
                started_at_ms: exact_ms(started_at_ms).unwrap_or(0),
            });
        }
        // Making it again drops a kept refusal: its outcome is this job's.
        kept_refusals(inner, config, |kept| refusals::cleared(kept, location));
        (inner.changed)();
        let job = RubricJob {
            job_id: job_id.clone(),
            location_text: location.to_owned(),
            location: skill.location,
            folder: skill.folder,
            existing,
            claude,
            environment,
            cancel,
        };
        let worker = Arc::clone(inner);
        let spawned = std::thread::Builder::new()
            .name("rubric-maker".into())
            .spawn(move || make_work(&worker, job));
        if spawned.is_err() {
            end_job(
                inner,
                &job_id,
                location,
                GradingRubricJob::Failed {
                    job_id: job_id.clone(),
                    failure: GradingRubricFailure::InternalError,
                },
                None,
            );
            return GradingRubricMake::Failed;
        }
        GradingRubricMake::Started { job_id }
    }

    /// The ID of the run going now, if any.
    pub fn active_run(&self) -> Option<String> {
        lock(&self.inner.active)
            .as_ref()
            .map(|active| active.run_id.clone())
    }

    /// Ask the run `run_id` to stop. False when it is not the run going now.
    pub fn cancel(&self, run_id: &str) -> bool {
        match lock(&self.inner.active).as_ref() {
            Some(active) if active.run_id == run_id => {
                active.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    /// Start grading `project_path` with the built-in or added skill
    /// `skill_id`. The run continues on its own thread; this returns once
    /// its `running` row is committed.
    pub fn start(&self, project_path: &str, skill_id: &str) -> GradingStart {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return GradingStart::NotConfigured;
        };
        if inner.stopped.load(Ordering::SeqCst) {
            return GradingStart::Stopped;
        }
        let (skill, kind) = if BUILTIN.contains(&skill_id) {
            let Some(skill) = open_bundled(config, skill_id) else {
                return GradingStart::SkillUnavailable;
            };
            (skill, SkillKind::BuiltIn)
        } else {
            let Ok(added) = store_call(&*inner.store, |store| store.added_skills()) else {
                return GradingStart::Failed;
            };
            let Some(added) = added.iter().find(|skill| skill.skill_id == skill_id) else {
                return GradingStart::UnknownSkill;
            };
            let Ok(skill) = open_added(config, added) else {
                return GradingStart::SkillUnavailable;
            };
            (skill, SkillKind::Added)
        };
        let Some(project) = listed_project(&config.projects, project_path) else {
            return GradingStart::NotAProject;
        };
        let environment = config.environment.read();
        let Ok(claude) = runner::find_claude(&environment) else {
            return GradingStart::ClaudeUnavailable;
        };
        let Some(run_id) = crate::telemetry::random_uuid() else {
            return GradingStart::Failed;
        };
        let cancel = CancelFlag::new();
        {
            // An added skill removed since it was read above is not run: a
            // removal holds `membership` across its delete, so this read,
            // made while holding it until the slot is claimed, sees it. The
            // slot's own lock is not held while storage is read. A built-in
            // skill cannot be removed and waits on no removal.
            let _membership = (kind == SkillKind::Added).then(|| lock(&inner.membership));
            if kind == SkillKind::Added {
                match store_call(&*inner.store, |store| store.added_skills()) {
                    Ok(added) if added.iter().any(|added| added.skill_id == skill_id) => {}
                    Ok(_) => return GradingStart::UnknownSkill,
                    Err(_) => return GradingStart::Failed,
                }
            }
            let mut active = lock(&inner.active);
            if active.is_some() {
                return GradingStart::Busy;
            }
            // Checked again under the slot: shutdown sets the flag and then
            // takes this lock, so it either sees this run or refuses it.
            if inner.stopped.load(Ordering::SeqCst) {
                return GradingStart::Stopped;
            }
            *active = Some(Active {
                run_id: run_id.clone(),
                skill_id: Some(skill_id.to_owned()),
                cancel: cancel.clone(),
            });
        }
        let started_at_ms = inner.now();
        let saved = store_call(&*inner.store, |store| {
            store.grade_insert_running(&NewGradeRun {
                run_id: &run_id,
                project_path,
                skill_id,
                skill_version: &skill.version(),
                started_at_ms,
            })
        });
        if saved.is_err() {
            release(inner);
            return GradingStart::Failed;
        }
        (inner.changed)();
        let job = Job {
            run_id: run_id.clone(),
            project_path: project_path.to_owned(),
            project,
            skill,
            kind,
            claude,
            environment,
            cancel,
        };
        let worker = Arc::clone(inner);
        let spawned = std::thread::Builder::new()
            .name("grading-run".into())
            .spawn(move || work(&worker, job));
        if spawned.is_err() {
            finish(
                inner,
                &run_id,
                &Outcome::Failed(GradingFailure::InternalError),
            );
            release(inner);
            return GradingStart::Failed;
        }
        GradingStart::Started { run_id }
    }

    /// Refuse every later run, cancel the one going and wait, within the
    /// bound, until it is saved and its staging folder removed. False when
    /// the wait ran out (the next start then marks the row interrupted).
    pub fn shutdown(&self) -> bool {
        shutdown(&self.inner)
    }

    /// [`Self::shutdown`] on its own thread, so quitting can stop the other
    /// services meanwhile; join it before storage closes. `None` when no
    /// thread could be started (then call [`Self::shutdown`]).
    pub fn shutdown_in_background(&self) -> Option<std::thread::JoinHandle<bool>> {
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("grading-shutdown".into())
            .spawn(move || shutdown(&inner))
            .ok()
    }

    /// After a start: mark every run still `running` interrupted
    /// (`app_closed`) and remove the staging folders this database's runs
    /// left, only when their parent is a real folder this user owns and
    /// only it can open. Before a folder goes, the Claude Code process it
    /// recorded is stopped if that exact process is still running (waiting
    /// at most the runner's `SIGTERM` grace for it). Staging folders it has
    /// no row for (another copy of the app's) are left alone, and so are
    /// their processes. Returns how many staging folders were removed.
    pub fn recover(&self) -> usize {
        let inner = &self.inner;
        let Some(config) = &inner.config else {
            return 0;
        };
        if let Ok(ids) = store_call(&*inner.store, |store| {
            store.grades_interrupt_running(inner.now(), GradingFailure::AppClosed.code())
        }) && !ids.is_empty()
        {
            (inner.changed)();
        }
        // Nothing is listed or removed under a parent that is a link, not
        // this user's, or open to others.
        let Some(parent) = private_parent(&config.temp_dir) else {
            return 0;
        };
        let active = self.active_run();
        // This app's rubric-making staging folders all belong to jobs of a
        // start that ended: none is going while nothing holds the slot.
        let makers = if active.is_none() {
            maker::remove_leftovers(
                &config.temp_dir,
                &config.maker_key,
                config.limits.term_grace,
            )
        } else {
            0
        };
        makers
            + remove_run_folders(&parent, |name| {
                let leftover = active.as_deref() != Some(name)
                    && store_call(&*inner.store, |store| store.grade(name))
                        .is_ok_and(|row| row.is_some());
                // A Claude Code the crash left running is stopped before its
                // folder goes, only while it is still the process launched.
                if leftover {
                    stop_recorded_leader(&parent.join(name), config.limits.term_grace);
                }
                leftover
            })
    }

    /// Run a content purge with the kept grade folders: no run's folder is
    /// written meanwhile, and after a purge that committed every kept folder
    /// is removed (their result and report hold the content the purge
    /// cleared from the rows), and so are the kept rubric-making refusals
    /// (in memory and their file, under their lock; a file that cannot be
    /// removed is emptied). The rows stay.
    pub fn purge_with_folders<T, E>(&self, purge: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let _folders = lock(&self.inner.folders);
        let outcome = purge();
        if outcome.is_ok()
            && let Some(config) = &self.inner.config
        {
            remove_run_folders(&config.grades_dir, |_| true);
            let mut kept = lock(&self.inner.refusals);
            *kept = Some(Vec::new());
            let store = RefusalStore::new(config.refusals.clone());
            if store.clear().is_err() {
                let _ = store.write(&[]);
            }
        }
        outcome
    }
}

impl Inner {
    fn now(&self) -> i64 {
        self.config
            .as_ref()
            .map_or_else(now_ms, |config| (config.clock)())
    }
}

fn shutdown(inner: &Inner) -> bool {
    inner.stopped.store(true, Ordering::SeqCst);
    let mut active = lock(&inner.active);
    if let Some(active) = active.as_ref() {
        active.cancel.cancel();
    }
    let bound = inner
        .config
        .as_ref()
        .map_or(Duration::ZERO, |config| config.shutdown_bound);
    let deadline = Instant::now() + bound;
    while active.is_some() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        active = inner
            .idle
            .wait_timeout(active, remaining)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0;
    }
    true
}

fn release(inner: &Inner) {
    lock(&inner.active).take();
    inner.idle.notify_all();
}

fn open_bundled(config: &GradingConfig, id: &str) -> Option<GradingSkill> {
    let dir = skill::bundled_dir(&config.skills_root, id).ok()?;
    GradingSkill::open_builtin(&dir, id).ok()
}

fn info(skill: &GradingSkill, origin: GradingSkillOrigin) -> GradingSkillInfo {
    GradingSkillInfo {
        id: skill.id().to_owned(),
        name: skill.name().to_owned(),
        version: skill.version(),
        description: skill.description().to_owned(),
        origin,
        problem: None,
    }
}

/// The rubrics XTrace keeps.
fn rubric_store(config: &GradingConfig) -> RubricStore {
    RubricStore::new(config.rubric_store.clone())
}

/// The installed skills under the native home, each with the rubric that
/// applies to it.
fn discover(config: &GradingConfig) -> installed::Discovery {
    installed::discover_with(&config.home, Some(&rubric_store(config)))
}

/// The skill in `folder` found at `location`, opened with the rubric that
/// applies to it ([`rubric_store::applicable`]), which must carry `id`.
fn open_located(
    config: &GradingConfig,
    folder: &Path,
    location: &Location,
    id: &str,
) -> Result<GradingSkill, xt_judge::SkillError> {
    let applies = rubric_store::applicable(folder, location, Some(&rubric_store(config)));
    GradingSkill::open_as_with(folder, Some(&applies.path), id)
}

/// The folder an added skill is in now ([`installed::locate`] under the
/// home) with its location, and the plugin it comes from, if any.
fn added_folder(
    home: &Path,
    added: &AddedSkill,
) -> (
    Result<(PathBuf, Location), GradingSkillProblem>,
    Option<String>,
) {
    let Some(location) = Location::parse(&added.location) else {
        return (Err(GradingSkillProblem::FolderMissing), None);
    };
    let plugin = location.plugin().map(str::to_owned);
    let folder = installed::locate(home, &location).ok_or(match location {
        Location::Folder(_) => GradingSkillProblem::FolderMissing,
        Location::Plugin { .. } => GradingSkillProblem::PluginUnavailable,
    });
    (folder.map(|folder| (folder, location)), plugin)
}

/// An added skill opened from its folder now: a real folder (not a link)
/// whose rubric still carries the id it was added with.
fn open_added(
    config: &GradingConfig,
    added: &AddedSkill,
) -> Result<GradingSkill, GradingSkillProblem> {
    let (folder, location) = added_folder(&config.home, added).0?;
    open_folder(config, &folder, &location, added)
}

fn open_folder(
    config: &GradingConfig,
    folder: &Path,
    location: &Location,
    added: &AddedSkill,
) -> Result<GradingSkill, GradingSkillProblem> {
    if !std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.is_dir()) {
        return Err(GradingSkillProblem::FolderMissing);
    }
    open_located(config, folder, location, &added.skill_id).map_err(GradingSkillProblem::of)
}

/// An added skill's info: read from its folder now, or as stored with why it
/// cannot be opened.
fn added_info(config: &GradingConfig, added: &AddedSkill) -> GradingSkillInfo {
    let (folder, plugin) = added_folder(&config.home, added);
    let origin = GradingSkillOrigin::Added {
        folder: folder
            .as_ref()
            .ok()
            .and_then(|(folder, _)| folder.to_str())
            .map(str::to_owned),
        plugin,
    };
    match folder.and_then(|(folder, location)| open_folder(config, &folder, &location, added)) {
        Ok(skill) => info(&skill, origin),
        Err(problem) => GradingSkillInfo {
            id: added.skill_id.clone(),
            name: added.name.clone(),
            version: added.rubric_version.clone(),
            description: added.description.clone(),
            origin,
            problem: Some(problem),
        },
    }
}

/// Whether a run's skill is bundled or added: how its folder is copied and
/// how its staged copy is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SkillKind {
    BuiltIn,
    Added,
}

impl SkillKind {
    fn copy_limits(self) -> CopyLimits {
        match self {
            Self::BuiltIn => CopyLimits::BUILT_IN,
            Self::Added => CopyLimits::INSTALLED,
        }
    }

    /// Open the staged copy: it must carry the run's skill id.
    fn reopen(self, dir: &Path, id: &str) -> Result<GradingSkill, xt_judge::SkillError> {
        match self {
            Self::BuiltIn => GradingSkill::open_builtin(dir, id),
            Self::Added => GradingSkill::open_as(dir, id),
        }
    }
}

/// The folder `path` names when it is a project in a fresh list read and
/// is given exactly as its resolved path: absolute, a real folder (not a
/// link), and equal to what the file system resolves it to.
fn listed_project(projects: &ReadProjects, path: &str) -> Option<PathBuf> {
    let ProjectList::Loaded(list) = projects() else {
        return None;
    };
    if !list.projects.iter().any(|project| project.path == path) {
        return None;
    }
    let given = Path::new(path);
    if !given.is_absolute() || !std::fs::symlink_metadata(given).ok()?.is_dir() {
        return None;
    }
    let resolved = std::fs::canonicalize(given).ok()?;
    // Compared as text: `Path` equality ignores a trailing slash and `.`.
    (resolved.to_str() == Some(path)).then_some(resolved)
}

/// A stored time a renderer reads exactly; `None` past what JavaScript holds.
pub(crate) fn exact_ms(at: i64) -> Option<i64> {
    (0..=MAX_EXACT).contains(&at).then_some(at)
}

/// A run's stored insights, as the validated result gave them; `None` when
/// there are none (a run that did not succeed, or after a purge) or the
/// stored text does not read.
pub(crate) fn stored_insights(json: Option<&str>) -> Option<Vec<GradingInsight>> {
    json.and_then(|json| serde_json::from_str::<Vec<GradingInsight>>(json).ok())
}

fn run_dto(row: GradeRow) -> Option<GradingRun> {
    Some(GradingRun {
        started_at_ms: exact_ms(row.started_at_ms)?,
        finished_at_ms: match row.finished_at_ms {
            Some(at) => Some(exact_ms(at)?),
            None => None,
        },
        insights: stored_insights(row.insights_json.as_deref()),
        failure: row.failure_reason.as_deref().map(GradingFailure::from_code),
        run_id: row.run_id,
        project_path: row.project_path,
        skill_id: row.skill_id,
        skill_version: row.skill_version,
        status: row.status.into(),
        score: row.score,
        complete: row.complete,
        summary: row.summary,
        report_file: row.report_file,
    })
}

/// One accepted run.
struct Job {
    run_id: String,
    project_path: String,
    project: PathBuf,
    skill: GradingSkill,
    kind: SkillKind,
    claude: PathBuf,
    environment: Vec<(OsString, OsString)>,
    cancel: CancelFlag,
}

/// How a run ended, before it is saved.
enum Outcome {
    /// Already saved with its kept folder.
    Saved,
    Failed(GradingFailure),
    Cancelled,
    Interrupted,
}

/// Releases the slot however the run's thread ends; a run that did not get
/// saved (a panic) is saved as failed first.
struct Release<'a> {
    inner: &'a Inner,
    run_id: &'a str,
    saved: bool,
}

impl Drop for Release<'_> {
    fn drop(&mut self) {
        if !self.saved {
            finish(
                self.inner,
                self.run_id,
                &Outcome::Failed(GradingFailure::InternalError),
            );
        }
        release(self.inner);
    }
}

fn work(inner: &Inner, job: Job) {
    let mut guard = Release {
        inner,
        run_id: &job.run_id,
        saved: false,
    };
    let Some(config) = &inner.config else {
        return;
    };
    let started = Instant::now();
    let report = |state| GradingProgress {
        run_id: job.run_id.clone(),
        state,
        elapsed_seconds: u32::try_from(started.elapsed().as_secs()).unwrap_or(u32::MAX),
    };
    (inner.progress)(&report(GradingProgressState::Running));
    let (stop, stopped) = mpsc::channel::<()>();
    let report = &report;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(config.progress_every)
            {
                (inner.progress)(&report(GradingProgressState::Running));
            }
        });
        let outcome = run_job(inner, config, &job);
        if !matches!(outcome, Outcome::Saved) {
            finish(inner, &job.run_id, &outcome);
        }
        guard.saved = true;
        prune(inner, config);
        drop(stop);
    });
    (inner.progress)(&report(GradingProgressState::Finished));
}

/// Stage, check, run and keep. A succeeded run is saved here, under the
/// folder lock, together with its kept folder.
fn run_job(inner: &Inner, config: &GradingConfig, job: &Job) -> Outcome {
    let stopped = || inner.stopped.load(Ordering::SeqCst);
    let failed = |error: &GradeError| match GradingFailure::of(error) {
        Some(reason) => Outcome::Failed(reason),
        None if stopped() => Outcome::Interrupted,
        None => Outcome::Cancelled,
    };
    #[cfg(test)]
    if let Some(hook) = &config.before_staging {
        hook();
    }
    let staging = match Staging::create_with(
        &config.temp_dir,
        &job.run_id,
        job.skill.dir(),
        job.kind.copy_limits(),
    ) {
        Ok(staging) => staging,
        Err(error) => return failed(&error),
    };
    // A rubric XTrace keeps for the skill goes into the copy; the copy is
    // then checked like any other below.
    if !job.skill.rubric_in_folder()
        && let Err(error) = staging.place_rubric(&job.skill.rubric_path())
    {
        return failed(&error);
    }
    let outcome = (|| {
        let skill = match job.kind.reopen(&staging.skill_dir(), job.skill.id()) {
            Ok(skill) => skill,
            Err(error) => return Outcome::Failed(GradingFailure::of_skill(error)),
        };
        // The run's row carries the version read at its start; the copy
        // must be that version, or the stored version would be wrong.
        if skill.version() != job.skill.version() {
            return Outcome::Failed(GradingFailure::SkillChanged);
        }
        // Right before launch: still a listed project, still the same folder.
        if listed_project(&config.projects, &job.project_path).as_deref()
            != Some(job.project.as_path())
        {
            return Outcome::Failed(GradingFailure::ProjectChanged);
        }
        let grade_dir = staging.grade_dir();
        let result = runner::run(
            &GradeRun {
                claude: &job.claude,
                skill: &skill,
                project: &job.project,
                grade_dir: &grade_dir,
                inherited_env: &job.environment,
                limits: config.limits,
                leader_record: Some(&staging.leader_record()),
            },
            Some(&job.cancel),
        );
        match result {
            Ok(result) => keep(inner, config, job, &staging, &result),
            Err(error) => failed(&error),
        }
    })();
    staging.remove();
    outcome
}

/// Copy what is kept and save the succeeded run, both under the folder lock.
fn keep(
    inner: &Inner,
    config: &GradingConfig,
    job: &Job,
    staging: &Staging,
    result: &GradeResult,
) -> Outcome {
    let _folders = lock(&inner.folders);
    let destination = config.grades_dir.join(&job.run_id);
    if staging.archive(result, &destination).is_err() {
        return Outcome::Failed(GradingFailure::ArchiveFailed);
    }
    let Ok(insights) = serde_json::to_string(&result.insights) else {
        let _ = std::fs::remove_dir_all(&destination);
        return Outcome::Failed(GradingFailure::InternalError);
    };
    let saved = store_call(&*inner.store, |store| {
        store.grade_finish(
            &job.run_id,
            &GradeFinish::Succeeded {
                finished_at_ms: inner.now(),
                score: result.score,
                complete: result.complete,
                summary: &result.summary,
                insights_json: &insights,
                report_file: result.report.as_deref(),
            },
        )
    });
    if saved.is_ok_and(|moved| moved) {
        (inner.changed)();
        Outcome::Saved
    } else {
        // Nothing points at the folder; the row is finished by the caller.
        let _ = std::fs::remove_dir_all(&destination);
        Outcome::Failed(GradingFailure::InternalError)
    }
}

/// Save a run that did not succeed and publish the change when it moved.
/// When that save fails, one fallback saves the run failed
/// (`internal_error`): while the app runs, no released slot leaves a row
/// `running` behind (the store keeps every finish time at or after the
/// start, so the clock cannot be the reason).
fn finish(inner: &Inner, run_id: &str, outcome: &Outcome) {
    let at = inner.now();
    let finish = match outcome {
        Outcome::Saved => return,
        Outcome::Failed(reason) => GradeFinish::Failed {
            finished_at_ms: at,
            reason: reason.code(),
        },
        Outcome::Cancelled => GradeFinish::Cancelled { finished_at_ms: at },
        Outcome::Interrupted => GradeFinish::Interrupted {
            finished_at_ms: at,
            reason: GradingFailure::AppClosed.code(),
        },
    };
    let saved =
        store_call(&*inner.store, |store| store.grade_finish(run_id, &finish)).or_else(|_| {
            store_call(&*inner.store, |store| {
                store.grade_finish(
                    run_id,
                    &GradeFinish::Failed {
                        finished_at_ms: inner.now(),
                        reason: GradingFailure::InternalError.code(),
                    },
                )
            })
        });
    if saved.is_ok_and(|moved| moved) {
        (inner.changed)();
    }
}

/// Remove the kept folders of all but the newest succeeded runs.
fn prune(inner: &Inner, config: &GradingConfig) {
    let Ok(keep) = store_call(&*inner.store, |store| {
        store.grade_newest_succeeded_ids(config.keep_folders)
    }) else {
        return;
    };
    let keep: HashSet<String> = keep.into_iter().collect();
    let _folders = lock(&inner.folders);
    remove_run_folders(&config.grades_dir, |name| !keep.contains(name));
}

/// One accepted rubric-making job.
struct RubricJob {
    job_id: String,
    location_text: String,
    location: Location,
    /// The skill's folder as discovery found it.
    folder: PathBuf,
    /// Why its current rubric is refused; none when it has none (a first
    /// making), some when it is made again.
    existing: Option<RubricError>,
    claude: PathBuf,
    environment: Vec<(OsString, OsString)>,
    cancel: CancelFlag,
}

/// Ends a rubric-making job however its thread ends: its outcome is
/// remembered (a job that got none, a panic, as an internal error), the
/// slot is released and the change published.
struct JobRelease<'a> {
    inner: &'a Inner,
    job: &'a RubricJob,
    outcome: Option<GradingRubricJob>,
    /// The skill's fingerprint when the job started, kept with a refusal.
    fingerprint: Option<String>,
}

impl Drop for JobRelease<'_> {
    fn drop(&mut self) {
        let outcome = self.outcome.take().unwrap_or(GradingRubricJob::Failed {
            job_id: self.job.job_id.clone(),
            failure: GradingRubricFailure::InternalError,
        });
        end_job(
            self.inner,
            &self.job.job_id,
            &self.job.location_text,
            outcome,
            self.fingerprint.take(),
        );
    }
}

/// The kept refusals, read from their file at first use, after `change`;
/// written back when `change` changed them. A file that cannot be written
/// leaves the change in memory until XTrace quits.
fn kept_refusals<T>(
    inner: &Inner,
    config: &GradingConfig,
    change: impl FnOnce(&mut Vec<Refusal>) -> T,
) -> T {
    let store = RefusalStore::new(config.refusals.clone());
    let mut kept = lock(&inner.refusals);
    let kept = kept.get_or_insert_with(|| store.read());
    let before = kept.clone();
    let answer = change(kept);
    if *kept != before {
        let _ = store.write(kept);
    }
    answer
}

/// Remember a job's outcome (a refusal with the skill's `fingerprint` from
/// the job's start, kept across restarts), free the slot, then publish the
/// change.
fn end_job(
    inner: &Inner,
    job_id: &str,
    location: &str,
    outcome: GradingRubricJob,
    fingerprint: Option<String>,
) {
    if let (GradingRubricJob::Refused { reason, .. }, Some(config)) = (&outcome, &inner.config) {
        let refusal = Refusal {
            location: location.to_owned(),
            job_id: job_id.to_owned(),
            reason: reason.clone(),
            refused_at_ms: inner.now(),
            fingerprint: fingerprint.unwrap_or_default(),
        };
        kept_refusals(inner, config, |kept| refusals::recorded(kept, refusal));
    }
    lock(&inner.jobs).finish(job_id, location, outcome);
    release(inner);
    (inner.changed)();
}

fn make_work(inner: &Inner, job: RubricJob) {
    let mut guard = JobRelease {
        inner,
        job: &job,
        outcome: None,
        fingerprint: None,
    };
    let Some(config) = &inner.config else {
        return;
    };
    // The skill as the job reads it: a refusal holds only while it stays so.
    guard.fingerprint = refusals::fingerprint(&job.folder);
    let started = Instant::now();
    let report = |state| GradingRubricProgress {
        job_id: job.job_id.clone(),
        location: job.location_text.clone(),
        state,
        elapsed_seconds: u32::try_from(started.elapsed().as_secs()).unwrap_or(u32::MAX),
    };
    (inner.rubric_progress)(&report(GradingProgressState::Running));
    let (stop, stopped) = mpsc::channel::<()>();
    let report = &report;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(config.progress_every)
            {
                (inner.rubric_progress)(&report(GradingProgressState::Running));
            }
        });
        guard.outcome = Some(make_job(inner, config, &job));
        drop(stop);
    });
    drop(guard);
    (inner.rubric_progress)(&report(GradingProgressState::Finished));
}

/// Stage, run, check, settle the id and save. Only this writes the rubric,
/// and only after the reader accepted it.
fn make_job(inner: &Inner, config: &GradingConfig, job: &RubricJob) -> GradingRubricJob {
    let failed = |failure| GradingRubricJob::Failed {
        job_id: job.job_id.clone(),
        failure,
    };
    let cancelled = || GradingRubricJob::Cancelled {
        job_id: job.job_id.clone(),
    };
    let run_failed = |error: &GradeError| match GradingFailure::of(error) {
        Some(failure) => failed(GradingRubricFailure::Run { failure }),
        None => cancelled(),
    };
    // The skill's real folder, through any link on its way.
    let Some(folder) = std::fs::canonicalize(&job.folder)
        .ok()
        .filter(|folder| folder.is_dir())
    else {
        return failed(GradingRubricFailure::SkillChanged);
    };
    // Made again: the refused rubric that applies now, which the staged copy
    // carries in place of the skill's own entry, and whose version the new
    // one follows.
    let refused = job.existing.map(|_| {
        rubric_store::applicable(&folder, &job.location, Some(&rubric_store(config))).path
    });
    let written = maker::make(
        &maker::Job {
            claude: &job.claude,
            temp_dir: &config.temp_dir,
            key: &config.maker_key,
            run_id: &job.job_id,
            skill: &folder,
            helper: &config.maker_dir,
            replaced: job
                .existing
                .zip(refused.as_deref())
                .map(|(error, path)| maker::Replaced { error, path }),
            inherited_env: &job.environment,
            limits: config.maker_limits,
        },
        Some(&job.cancel),
    );
    let made = match written {
        Ok(Written::Rubric(made)) => made,
        // A cancel after the run is still a cancel.
        Ok(Written::Refused(_)) if job.cancel.is_cancelled() => return cancelled(),
        Ok(Written::Refused(refusal)) => {
            return GradingRubricJob::Refused {
                job_id: job.job_id.clone(),
                reason: refusal.reason,
            };
        }
        Err(MakeError::HelperUnavailable) => {
            return failed(GradingRubricFailure::HelperUnavailable);
        }
        Err(MakeError::Run(error)) => return run_failed(&error),
        Err(MakeError::Rubric(RubricError::Missing)) => {
            return failed(GradingRubricFailure::RubricMissing);
        }
        Err(MakeError::Rubric(problem)) => {
            return failed(GradingRubricFailure::RubricInvalid {
                rubric: problem.into(),
            });
        }
        Err(MakeError::Refusal) => return failed(GradingRubricFailure::RefusalUnreadable),
        Err(MakeError::Both) => return failed(GradingRubricFailure::BothWritten),
    };
    // A cancel after the run is still a cancel: nothing is saved.
    if job.cancel.is_cancelled() {
        return cancelled();
    }
    let (bytes, rubric) = match settle_id(inner, config, job, &made.bytes, &made.rubric) {
        Ok(settled) => settled,
        Err(failure) => return failed(failure),
    };
    #[cfg(test)]
    if let Some(hook) = &config.before_save {
        hook();
    }
    // Checked again under the slot, which a cancel takes to set its flag: a
    // job cancelled by now saves nothing, and none is cancelled while it
    // saves.
    let active = lock(&inner.active);
    if job.cancel.is_cancelled() {
        return cancelled();
    }
    let saved = save_made(config, job, &folder, &bytes);
    drop(active);
    match saved {
        Ok(kept_by_xtrace) => {
            (inner.changed)();
            GradingRubricJob::Made {
                job_id: job.job_id.clone(),
                criteria: rubric
                    .criteria
                    .iter()
                    .map(|criterion| GradingRubricCriterion {
                        name: criterion.name.clone(),
                        points: criterion.points,
                    })
                    .collect(),
                kept_by_xtrace,
            }
        }
        Err(failure) => failed(failure),
    }
}

/// The made rubric with an id no other skill has. Remaking the rubric of a
/// skill added to Scores keeps the id its card has. Otherwise the made id
/// stays unless a built-in skill, an added skill or another installed
/// skill whose rubric is ready has it; then it gets the smallest free
/// suffix ([`maker::free_id`]), or the job fails when none is free.
fn settle_id(
    inner: &Inner,
    config: &GradingConfig,
    job: &RubricJob,
    bytes: &[u8],
    made: &Rubric,
) -> Result<(Vec<u8>, Rubric), GradingRubricFailure> {
    let invalid = |problem: RubricError| GradingRubricFailure::RubricInvalid {
        rubric: problem.into(),
    };
    let added = store_call(&*inner.store, |store| store.added_skills())
        .map_err(|_| GradingRubricFailure::InternalError)?;
    if let Some(own) = added
        .iter()
        .find(|added| added.location == job.location_text)
    {
        return rubric::with_id(bytes, &own.skill_id).map_err(invalid);
    }
    let mut taken: HashSet<String> = BUILTIN.iter().map(|id| (*id).to_owned()).collect();
    taken.extend(added.into_iter().map(|added| added.skill_id));
    taken.extend(
        discover(config)
            .skills
            .into_iter()
            .filter(|skill| skill.location.text().as_deref() != Some(job.location_text.as_str()))
            .filter_map(|skill| match skill.rubric {
                RubricStatus::Ready(rubric) => Some(rubric.id),
                _ => None,
            }),
    );
    let id = maker::free_id(&made.id, &taken).ok_or(GradingRubricFailure::IdUnavailable)?;
    if id == made.id {
        Ok((bytes.to_vec(), made.clone()))
    } else {
        rubric::with_id(bytes, &id).map_err(invalid)
    }
}

/// Save a made rubric for the skill in the resolved `folder`. Returns
/// whether it went to XTrace's rubric store.
///
/// * Made again (the skill's rubric had a problem) and the refused rubric is
///   still where the one rule finds it ([`rubric_store::applicable`]): it is
///   replaced there, the old file moved to `xtrace-rubric.md.bak`. A skill
///   folder that cannot be written then fails the job
///   ([`GradingRubricFailure::FolderNotWritable`]): its own rubric would
///   still win over one in the store.
/// * Otherwise nothing may be there yet (a rubric that appeared during a
///   first making is not replaced: [`GradingRubricFailure::SkillChanged`]).
///   A skill folder's skill gets it next to its `SKILL.md`, or, when the
///   folder cannot be written, in the store; a plugin's skill always in the
///   store, keyed by its plugin and name.
fn save_made(
    config: &GradingConfig,
    job: &RubricJob,
    folder: &Path,
    bytes: &[u8],
) -> Result<bool, GradingRubricFailure> {
    let store = rubric_store(config);
    let failure = |error: SaveError| match error {
        SaveError::Exists => GradingRubricFailure::SkillChanged,
        SaveError::NotWritable | SaveError::Failed => GradingRubricFailure::SaveFailed,
    };
    if !folder.is_dir() {
        return Err(GradingRubricFailure::SkillChanged);
    }
    let applies = rubric_store::applicable(folder, &job.location, Some(&store));
    if std::fs::symlink_metadata(&applies.path).is_ok() {
        if job.existing.is_none() {
            return Err(GradingRubricFailure::SkillChanged);
        }
        if applies.kept_by_xtrace {
            let kept = applies
                .path
                .parent()
                .ok_or(GradingRubricFailure::SaveFailed)?;
            rubric_store::save(kept, bytes, true, &job.job_id, STORE_RUBRIC_MODE)
                .map_err(failure)?;
            return Ok(true);
        }
        return match rubric_store::save(folder, bytes, true, &job.job_id, FOLDER_RUBRIC_MODE) {
            Ok(()) => Ok(false),
            Err(SaveError::NotWritable) => Err(GradingRubricFailure::FolderNotWritable),
            Err(error) => Err(failure(error)),
        };
    }
    if matches!(job.location, Location::Folder(_)) {
        match rubric_store::save(folder, bytes, false, &job.job_id, FOLDER_RUBRIC_MODE) {
            Ok(()) => return Ok(false),
            // Kept by XTrace instead.
            Err(SaveError::NotWritable) => {}
            Err(error) => return Err(failure(error)),
        }
    }
    let kept = rubric_store::make_store_folder(&store, &job.location)
        .map_err(|_| GradingRubricFailure::SaveFailed)?;
    rubric_store::save(&kept, bytes, false, &job.job_id, STORE_RUBRIC_MODE).map_err(failure)?;
    Ok(true)
}

#[cfg(test)]
#[path = "grading_tests.rs"]
mod tests;
