//! The private staging folder a run happens in and what is kept of it.
#![cfg(unix)]
//!
//! Everything happens under a fresh folder in Cargo's target temp folder,
//! which stands in for the system temporary folder and for the app's data
//! folder. No run is started here.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use xt_judge::{
    CopyLimits, GradeError, GradeResult, Staging,
    staging::{self, STAGING_DIR, is_run_id, remove_run_folders},
};

const RUN: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const OTHER: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

fn scratch(prefix: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn vendor_skill() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/claude-md-doctor/skill"
    ))
}

fn result(report: Option<&str>) -> GradeResult {
    GradeResult {
        score: 80,
        summary: "s".into(),
        insights: vec![],
        report: report.map(str::to_owned),
        complete: true,
        criteria: None,
    }
}

fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn staging_is_private_has_no_whitespace_and_holds_a_working_skill_copy() {
    let (_dir, tmp) = scratch("tmp-");
    let staging = Staging::create(&tmp, RUN, &vendor_skill()).unwrap();
    assert_eq!(staging.root(), tmp.join(STAGING_DIR).join(RUN));
    assert!(
        !staging
            .root()
            .to_str()
            .unwrap()
            .contains(char::is_whitespace)
    );
    for folder in [tmp.join(STAGING_DIR), staging.root().to_path_buf()] {
        let mode = fs::metadata(&folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "{}", folder.display());
    }
    assert!(fs::read_dir(staging.grade_dir()).unwrap().next().is_none());
    let skill = xt_judge::GradingSkill::open(&staging.skill_dir()).unwrap();
    assert_eq!(skill.id(), "claude-md-doctor");
    // The staged rubric is the bundled one, byte for byte.
    assert_eq!(
        skill.version(),
        xt_judge::GradingSkill::open(&vendor_skill())
            .unwrap()
            .version()
    );
    assert_eq!(
        fs::read(staging.skill_dir().join("SKILL.md")).unwrap(),
        fs::read(vendor_skill().join("SKILL.md")).unwrap()
    );
    let root = staging.root().to_path_buf();
    assert!(staging.remove());
    assert!(!root.exists());
    assert!(
        tmp.join(STAGING_DIR).is_dir(),
        "only the run's own folder goes"
    );
}

#[test]
fn dropping_a_staging_removes_it() {
    let (_dir, tmp) = scratch("tmp-");
    let root = Staging::create(&tmp, RUN, &vendor_skill())
        .unwrap()
        .root()
        .to_path_buf();
    assert!(!root.exists());
}

#[test]
fn a_temporary_folder_with_whitespace_is_refused_with_the_runners_reason() {
    let (_dir, tmp) = scratch("tmp-");
    let spaced = tmp.join("with space");
    fs::create_dir(&spaced).unwrap();
    assert_eq!(
        Staging::create(&spaced, RUN, &vendor_skill()).unwrap_err(),
        GradeError::GradeDirPathHasWhitespace
    );
    assert!(!spaced.join(STAGING_DIR).join(RUN).exists());
}

#[test]
fn an_unsafe_parent_an_existing_run_folder_or_a_bad_id_is_refused() {
    let (_dir, tmp) = scratch("tmp-");
    assert_eq!(
        Staging::create(&tmp, "not-a-run-id", &vendor_skill()).unwrap_err(),
        GradeError::StagingFailed
    );
    // A parent others can open.
    fs::create_dir(tmp.join(STAGING_DIR)).unwrap();
    fs::set_permissions(tmp.join(STAGING_DIR), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        Staging::create(&tmp, RUN, &vendor_skill()).unwrap_err(),
        GradeError::StagingFailed
    );
    fs::set_permissions(tmp.join(STAGING_DIR), fs::Permissions::from_mode(0o700)).unwrap();
    // A run folder that already exists is never reused.
    fs::create_dir(tmp.join(STAGING_DIR).join(RUN)).unwrap();
    fs::write(tmp.join(STAGING_DIR).join(RUN).join("keep"), "x").unwrap();
    assert_eq!(
        Staging::create(&tmp, RUN, &vendor_skill()).unwrap_err(),
        GradeError::StagingFailed
    );
    assert!(tmp.join(STAGING_DIR).join(RUN).join("keep").is_file());
    // A parent that is a link.
    let (_other, elsewhere) = scratch("tmp-");
    fs::create_dir(elsewhere.join("real")).unwrap();
    fs::set_permissions(elsewhere.join("real"), fs::Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::symlink(elsewhere.join("real"), elsewhere.join(STAGING_DIR)).unwrap();
    assert_eq!(
        Staging::create(&elsewhere, RUN, &vendor_skill()).unwrap_err(),
        GradeError::StagingFailed
    );
}

#[test]
fn a_skill_with_a_link_out_of_its_folder_is_not_staged() {
    let (_dir, tmp) = scratch("tmp-");
    let (_skill_dir, skill) = scratch("skill-");
    fs::write(skill.join("SKILL.md"), "x").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", skill.join("hosts")).unwrap();
    assert_eq!(
        Staging::create(&tmp, RUN, &skill).unwrap_err(),
        GradeError::SkillLinkRefused
    );
    assert!(!tmp.join(STAGING_DIR).join(RUN).exists());
}

#[test]
fn a_link_to_a_file_inside_the_skill_is_copied_as_that_file_and_no_other_link_is() {
    let (_dir, tmp) = scratch("tmp-");
    let (_skill_dir, skill) = scratch("skill-");
    fs::create_dir(skill.join("docs")).unwrap();
    fs::write(skill.join("SKILL.md"), "x").unwrap();
    fs::write(skill.join("docs/guide.md"), "guide").unwrap();
    std::os::unix::fs::symlink("docs/guide.md", skill.join("GUIDE.md")).unwrap();
    let staging = Staging::create_with(&tmp, RUN, &skill, CopyLimits::INSTALLED).unwrap();
    let copied = staging.skill_dir().join("GUIDE.md");
    assert!(fs::symlink_metadata(&copied).unwrap().is_file());
    assert_eq!(fs::read_to_string(copied).unwrap(), "guide");
    drop(staging);
    // A link to a folder inside, and a link to nothing, are refused.
    for (name, target) in [("folder", "docs"), ("dangling", "missing.md")] {
        let (_other_dir, other) = scratch("skill-");
        fs::create_dir(other.join("docs")).unwrap();
        fs::write(other.join("SKILL.md"), "x").unwrap();
        std::os::unix::fs::symlink(target, other.join(name)).unwrap();
        assert_eq!(
            Staging::create_with(&tmp, RUN, &other, CopyLimits::INSTALLED).unwrap_err(),
            GradeError::SkillLinkRefused,
            "{name}"
        );
        assert!(!tmp.join(STAGING_DIR).join(RUN).exists());
    }
}

#[test]
fn a_skill_past_its_copy_limits_is_refused_and_nothing_is_left() {
    let (_dir, tmp) = scratch("tmp-");
    let (_skill_dir, skill) = scratch("skill-");
    fs::write(skill.join("SKILL.md"), "12345").unwrap();
    fs::write(skill.join("two.md"), "12345").unwrap();
    fs::create_dir_all(skill.join("a/b")).unwrap();
    fs::write(skill.join("a/b/three.md"), "1").unwrap();
    // Hidden folders and __pycache__ are never copied and do not count.
    fs::create_dir(skill.join(".git")).unwrap();
    fs::write(skill.join(".git/huge"), vec![0_u8; 4096]).unwrap();
    // Three files and two folders.
    let fits = CopyLimits {
        files: 5,
        bytes: 11,
        depth: 2,
    };
    drop(Staging::create_with(&tmp, RUN, &skill, fits).unwrap());
    for limits in [
        CopyLimits { files: 4, ..fits },
        CopyLimits { bytes: 10, ..fits },
        CopyLimits { depth: 1, ..fits },
    ] {
        assert_eq!(
            Staging::create_with(&tmp, RUN, &skill, limits).unwrap_err(),
            GradeError::SkillTooLarge,
            "{limits:?}"
        );
        assert!(!tmp.join(STAGING_DIR).join(RUN).exists());
    }
    assert_eq!(
        (CopyLimits::INSTALLED.files, CopyLimits::INSTALLED.bytes),
        (2_000, 50 * 1024 * 1024)
    );
}

#[test]
fn folders_count_toward_the_limit_and_a_crowded_folder_is_not_read_through() {
    let (_dir, tmp) = scratch("tmp-");
    // Many empty folders and no file past SKILL.md: each folder counts.
    let (_skill_dir, skill) = scratch("skill-");
    fs::write(skill.join("SKILL.md"), "x").unwrap();
    for index in 0..CopyLimits::INSTALLED.files {
        fs::create_dir(skill.join(format!("empty-{index}"))).unwrap();
    }
    assert_eq!(
        Staging::create_with(&tmp, RUN, &skill, CopyLimits::INSTALLED).unwrap_err(),
        GradeError::SkillTooLarge
    );
    assert!(!tmp.join(STAGING_DIR).join(RUN).exists());
    // Nested empty folders count the same way.
    let (_nested_dir, nested) = scratch("skill-");
    fs::write(nested.join("SKILL.md"), "x").unwrap();
    for index in 0..4 {
        fs::create_dir_all(nested.join(format!("a{index}/b"))).unwrap();
    }
    let limits = CopyLimits {
        files: 8,
        ..CopyLimits::INSTALLED
    };
    assert_eq!(
        Staging::create_with(&tmp, RUN, &nested, limits).unwrap_err(),
        GradeError::SkillTooLarge
    );
    drop(Staging::create_with(&tmp, RUN, &nested, CopyLimits { files: 9, ..limits }).unwrap());
    // A folder with more entries than the bound is refused even when they
    // are all hidden ones that would never be copied.
    let (_hidden_dir, hidden) = scratch("skill-");
    fs::write(hidden.join("SKILL.md"), "x").unwrap();
    for index in 0..10 {
        fs::write(hidden.join(format!(".cache-{index}")), "").unwrap();
    }
    let crowded = CopyLimits {
        files: 10,
        ..CopyLimits::INSTALLED
    };
    assert_eq!(
        Staging::create_with(&tmp, RUN, &hidden, crowded).unwrap_err(),
        GradeError::SkillTooLarge
    );
    drop(
        Staging::create_with(
            &tmp,
            RUN,
            &hidden,
            CopyLimits {
                files: 11,
                ..crowded
            },
        )
        .unwrap(),
    );
}

/// A grade folder as a finished run leaves it: result, report, cards, the
/// skill's own report.json and proposals, and a work folder with excerpts.
fn finished(staging: &Staging) {
    let grade = staging.grade_dir();
    fs::write(grade.join("result.json"), "{}").unwrap();
    fs::write(grade.join("report.html"), "<p>r</p>").unwrap();
    fs::write(grade.join("card.svg"), "<svg/>").unwrap();
    fs::write(grade.join("claude-md-health.svg"), "<svg/>").unwrap();
    fs::write(grade.join("report.json"), "{}").unwrap();
    fs::write(grade.join("proposal.md"), "p").unwrap();
    fs::create_dir(grade.join("work")).unwrap();
    fs::write(grade.join("work/excerpts.jsonl"), "conversation text").unwrap();
    fs::write(grade.join("work/card.svg"), "<svg/>").unwrap();
}

#[test]
fn archive_keeps_only_the_result_the_report_and_its_cards_never_work() {
    let (_dir, tmp) = scratch("tmp-");
    let (_data, data) = scratch("data with space-");
    let staging = Staging::create(&tmp, RUN, &vendor_skill()).unwrap();
    finished(&staging);
    let destination = data.join("grades").join(RUN);
    staging
        .archive(&result(Some("report.html")), &destination)
        .unwrap();
    assert_eq!(
        files_under(&destination),
        [
            "card.svg",
            "claude-md-health.svg",
            "report.html",
            "result.json"
        ]
    );
    // A second archive into the same place is refused, never merged.
    assert_eq!(
        staging.archive(&result(Some("report.html")), &destination),
        Err(GradeError::ArchiveFailed)
    );
}

#[test]
fn archive_without_a_report_keeps_the_result_alone() {
    let (_dir, tmp) = scratch("tmp-");
    let (_data, data) = scratch("data-");
    let staging = Staging::create(&tmp, RUN, &vendor_skill()).unwrap();
    finished(&staging);
    let destination = data.join(RUN);
    staging.archive(&result(None), &destination).unwrap();
    assert_eq!(files_under(&destination), ["result.json"]);
}

#[test]
fn archive_refuses_a_report_in_work_or_through_a_link_and_leaves_nothing() {
    let (_dir, tmp) = scratch("tmp-");
    let (_data, data) = scratch("data-");
    let staging = Staging::create(&tmp, RUN, &vendor_skill()).unwrap();
    finished(&staging);
    fs::write(staging.grade_dir().join("work/report.html"), "x").unwrap();
    std::os::unix::fs::symlink(
        staging.grade_dir().join("work/report.html"),
        staging.grade_dir().join("linked.html"),
    )
    .unwrap();
    for report in ["work/report.html", "WORK/report.html", "linked.html"] {
        let destination = data.join(RUN);
        assert_eq!(
            staging.archive(&result(Some(report)), &destination),
            Err(GradeError::ArchiveFailed),
            "{report}"
        );
        assert!(!destination.exists(), "{report}");
    }
}

#[test]
fn only_selected_run_id_entries_are_removed() {
    let (_dir, parent) = scratch("runs-");
    let (_outside, outside) = scratch("outside-");
    fs::write(outside.join("precious"), "x").unwrap();
    for name in [RUN, OTHER] {
        fs::create_dir(parent.join(name)).unwrap();
        fs::write(parent.join(name).join("f"), "x").unwrap();
    }
    fs::create_dir(parent.join("not-a-run")).unwrap();
    let linked = "11111111-2222-4333-8444-555555555555";
    std::os::unix::fs::symlink(&outside, parent.join(linked)).unwrap();
    assert!(is_run_id(linked));
    let removed = remove_run_folders(&parent, |name| name != OTHER);
    assert_eq!(removed, 2);
    assert!(!parent.join(RUN).exists());
    assert!(parent.join(OTHER).join("f").is_file());
    assert!(parent.join("not-a-run").is_dir());
    assert!(fs::symlink_metadata(parent.join(linked)).is_err());
    assert!(
        outside.join("precious").is_file(),
        "a link's target is kept"
    );
    assert_eq!(remove_run_folders(&parent.join("missing"), |_| true), 0);
    assert_eq!(staging::staging_parent(&parent), parent.join(STAGING_DIR));
}

/// A leftover run folder that is a link is never followed: the record it
/// would name stops nothing. The same record in a real folder does.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_linked_run_folder_stops_nothing() {
    use std::time::Duration;
    use xt_probes::leader::{GroupLeader, GroupStop};
    let (_dir, root) = scratch("linked-run");
    let elsewhere = root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let parent = root.join(STAGING_DIR);
    fs::create_dir(&parent).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("30").stdin(std::process::Stdio::null());
    let mut child = xt_probes::process::spawn_grouped(&mut command).unwrap();
    let leader = GroupLeader::observe(child.id()).expect("a group leader");
    fs::write(elsewhere.join(staging::LEADER_RECORD), leader.record()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, parent.join(RUN)).unwrap();
    assert_eq!(
        staging::stop_recorded_leader(&parent.join(RUN), Duration::from_millis(200)),
        GroupStop::NotRunning
    );
    assert!(leader.is_running(), "nothing was signalled");
    assert_eq!(child.try_wait().unwrap(), None);
    // The record itself is good: read from a real folder, it stops the run.
    assert_ne!(
        staging::stop_recorded_leader(&elsewhere, Duration::from_secs(5)),
        GroupStop::NotRunning
    );
    child.wait().unwrap();
}
