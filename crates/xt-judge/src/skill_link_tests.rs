use super::*;
use std::io::Write;

/// The top folder of a made-up GitHub archive.
const TOP: &str = "octo-skills-0123456";

fn rubric_text(id: &str) -> String {
    format!(
        "---\nprotocol: 1\nid: {id}\nname: {id} check\nversion: 1\n\
         description: Checks {id}.\ntarget: project\n---\n## All (100 points)\nAll.\n"
    )
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nDo the thing.\n")
}

/// A tar archive written header by header, so tests can make entries no
/// well-behaved writer would.
#[derive(Default)]
pub(crate) struct Tar {
    bytes: Vec<u8>,
}

impl Tar {
    pub(crate) fn entry(
        mut self,
        path: &str,
        kind: u8,
        data: &[u8],
        link: &str,
        mode: u32,
    ) -> Self {
        let mut header = [0_u8; 512];
        let put = |field: &mut [u8], value: &[u8]| field[..value.len()].copy_from_slice(value);
        let octal = |field: &mut [u8], value: u64| {
            let text = format!("{value:0width$o}\0", width = field.len() - 1);
            field.copy_from_slice(text.as_bytes());
        };
        put(&mut header[0..100], path.as_bytes());
        octal(&mut header[100..108], u64::from(mode));
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], data.len() as u64);
        octal(&mut header[136..148], 0);
        header[148..156].fill(b' ');
        header[156] = kind;
        put(&mut header[157..257], link.as_bytes());
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|b| u32::from(*b)).sum();
        let text = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(text.as_bytes());
        self.bytes.extend_from_slice(&header);
        self.bytes.extend_from_slice(data);
        let pad = (512 - data.len() % 512) % 512;
        self.bytes.extend(std::iter::repeat_n(0, pad));
        self
    }

    pub(crate) fn dir(self, path: &str) -> Self {
        self.entry(&format!("{TOP}/{path}/"), b'5', &[], "", 0o755)
    }

    pub(crate) fn file(self, path: &str, data: &str) -> Self {
        self.entry(&format!("{TOP}/{path}"), b'0', data.as_bytes(), "", 0o644)
    }

    pub(crate) fn link(self, path: &str, target: &str) -> Self {
        self.entry(&format!("{TOP}/{path}"), b'2', &[], target, 0o777)
    }

    pub(crate) fn skill(self, path: &str, name: &str, rubric: Option<&str>) -> Self {
        let tar = self.file(
            &format!("{path}/SKILL.md"),
            &skill_md(name, &format!("The {name} skill.")),
        );
        match rubric {
            Some(id) => tar.file(&format!("{path}/xtrace-rubric.md"), &rubric_text(id)),
            None => tar,
        }
    }

    /// The archive as GitHub serves it: a global header naming the commit,
    /// the top folder, then the entries, gzip-compressed.
    pub(crate) fn gz(self) -> Vec<u8> {
        let mut tar = Tar::default()
            .entry(
                "pax_global_header",
                b'g',
                b"52 comment=0123456789abcdef0123456789abcdef01234567\n",
                "",
                0o666,
            )
            .entry(&format!("{TOP}/"), b'5', &[], "", 0o755);
        tar.bytes.extend_from_slice(&self.bytes);
        tar.raw_gz()
    }

    /// The entries as they are, with the end blocks, gzip-compressed.
    pub(crate) fn raw_gz(mut self) -> Vec<u8> {
        self.bytes.extend(std::iter::repeat_n(0, 1024));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&self.bytes).unwrap();
        encoder.finish().unwrap()
    }
}

fn staging() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let into = temp.path().join("stage");
    std::fs::create_dir(&into).unwrap();
    (temp, into)
}

fn unpack_all(archive: &[u8], prefix: &str) -> (tempfile::TempDir, Result<Unpacked, ArchiveError>) {
    let (temp, into) = staging();
    let result = unpack(
        archive,
        &into,
        prefix,
        ArchiveLimits::default(),
        &CancelFlag::new(),
    );
    (temp, result)
}

fn link(owner: &str, repo: &str, reference: Option<&str>, path: &str) -> GithubLink {
    GithubLink {
        owner: owner.into(),
        repo: repo.into(),
        reference: reference.map(str::to_owned),
        path: path.into(),
    }
}

#[test]
fn every_accepted_link_form_reads_as_one_repository_ref_and_folder() {
    for (text, expected) in [
        (
            "https://github.com/octo/skills",
            link("octo", "skills", None, ""),
        ),
        (
            "https://github.com/octo/skills/",
            link("octo", "skills", None, ""),
        ),
        (
            "https://github.com/octo/skills.git",
            link("octo", "skills", None, ""),
        ),
        (
            "  https://GitHub.com/octo/skills.git/ ",
            link("octo", "skills", None, ""),
        ),
        (
            "https://www.github.com/octo/skills",
            link("octo", "skills", None, ""),
        ),
        (
            "https://github.com/octo/skills/tree/main",
            link("octo", "skills", Some("main"), ""),
        ),
        (
            "https://github.com/octo/skills/tree/v1.2.0/skills/pdf/",
            link("octo", "skills", Some("v1.2.0"), "skills/pdf"),
        ),
        (
            "https://github.com/octo/skills/tree/0123456789abcdef0123456789abcdef01234567/a%20b",
            link(
                "octo",
                "skills",
                Some("0123456789abcdef0123456789abcdef01234567"),
                "a b",
            ),
        ),
        (
            "https://github.com/octo/skills/blob/main/skills/pdf/SKILL.md",
            link("octo", "skills", Some("main"), "skills/pdf"),
        ),
        (
            "https://github.com/octo/skills/blob/main/SKILL.md",
            link("octo", "skills", Some("main"), ""),
        ),
        (
            "https://github.com/my-org/repo_name.v2/tree/feature-x",
            link("my-org", "repo_name.v2", Some("feature-x"), ""),
        ),
    ] {
        assert_eq!(parse(text), Ok(expected), "{text}");
    }
    assert_eq!(
        parse("https://github.com/octo/skills")
            .unwrap()
            .repository(),
        "github.com/octo/skills"
    );
}

#[test]
fn anything_else_is_refused_with_its_reason() {
    for text in [
        "",
        "gitlab.com/octo/skills",
        "not a link",
        "http://github.com/octo/skills",
        "https://gitlab.com/octo/skills",
        "https://github.com.evil.example/octo/skills",
        "https://user@github.com/octo/skills",
        "https://github.com:443/octo/skills",
        "ftp://github.com/octo/skills",
    ] {
        assert_eq!(parse(text), Err(LinkError::NotGithub), "{text}");
    }
    for text in [
        "https://github.com",
        "https://github.com/",
        "https://github.com/octo",
        "https://github.com/octo/skills/issues/4",
        "https://github.com/octo/skills/pulls",
        "https://github.com/octo/skills/tree",
        "https://github.com/octo/skills/blob/main/README.md",
        "https://github.com/octo/skills/blob/main",
        "https://github.com?tab=readme",
        "https://github.com/octo//skills",
        "https://github.com/octo/skills/tree/main//pdf",
        "https://github.com/octo/skills//",
        "https://github.com/octo/../skills",
    ] {
        assert_eq!(parse(text), Err(LinkError::Shape), "{text}");
    }
    for text in [
        "https://github.com/-octo/skills",
        "https://github.com/octo_x/skills",
        "https://github.com/octo/sk%20ills",
        "https://github.com/octo/..",
        "https://github.com/octo/skills/tree/../x",
        "https://github.com/octo/skills/tree/-x",
        "https://github.com/octo/skills/tree/a..b",
        "https://github.com/octo/skills/tree/x.lock",
        "https://github.com/octo/skills/tree/main/..",
        "https://github.com/octo/skills/tree/main/a/%2E%2E",
        "https://github.com/octo/skills/tree/main/a%2Fb",
        "https://github.com/octo/skills/tree/main/%ZZ",
        "https://github.com/octo/skills/tree/main/%FF",
        "https://github.com/octo/skills/tree/main/a%00",
    ] {
        assert_eq!(parse(text), Err(LinkError::Name), "{text}");
    }
    let deep = format!(
        "https://github.com/octo/skills/tree/main/{}",
        vec!["a"; MAX_LINK_SEGMENTS + 1].join("/")
    );
    assert_eq!(parse(&deep), Err(LinkError::Name));
    let long = format!("https://github.com/octo/{}", "a".repeat(MAX_LINK_BYTES));
    assert_eq!(parse(&long), Err(LinkError::Shape));
}

#[test]
fn commits_are_full_lowercase_hex_and_shown_short() {
    assert!(is_commit("0123456789abcdef0123456789abcdef01234567"));
    assert!(!is_commit("0123456789ABCDEF0123456789abcdef01234567"));
    assert!(!is_commit("0123456"));
    assert!(!is_commit("main"));
    assert_eq!(short_commit("0123456789abcdef"), "0123456");
}

#[test]
fn unpacking_writes_only_the_links_folder_with_modes_and_records_links() {
    let archive = Tar::default()
        .dir("skills")
        .skill("skills/pdf", "pdf", Some("pdf"))
        .entry(
            &format!("{TOP}/skills/pdf/scripts/run.sh"),
            b'0',
            b"#!/bin/sh\n",
            "",
            0o755,
        )
        .link("skills/pdf/alias.md", "SKILL.md")
        .file("README.md", "# Skills\n")
        .skill("other/x", "x", None)
        .gz();
    let (_temp, unpacked) = unpack_all(&archive, "skills/pdf");
    let unpacked = unpacked.unwrap();
    let root = &unpacked.root;
    assert!(root.join("skills/pdf/SKILL.md").is_file());
    assert!(!root.join("README.md").exists());
    assert!(!root.join("other").exists());
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &str| {
        std::fs::metadata(root.join(path))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode("skills/pdf/scripts/run.sh"), 0o755);
    assert_eq!(mode("skills/pdf/SKILL.md"), 0o644);
    // The link is recorded, not created.
    assert!(std::fs::symlink_metadata(root.join("skills/pdf/alias.md")).is_err());
    assert_eq!(
        unpacked.links,
        vec![StagedLink {
            path: "skills/pdf/alias.md".into(),
            target: "SKILL.md".into(),
        }]
    );
    // The whole repository at its top.
    let (_temp, unpacked) = unpack_all(&archive, "");
    assert!(unpacked.unwrap().root.join("README.md").is_file());
}

#[test]
fn archives_past_their_bounds_are_refused() {
    let (_temp, into) = staging();
    let many = (0..20).fold(Tar::default(), |tar, at| tar.file(&format!("f{at}"), "x"));
    let limits = ArchiveLimits {
        entries: 10,
        bytes: MAX_UNPACKED_BYTES,
    };
    assert_eq!(
        unpack(many.gz().as_slice(), &into, "", limits, &CancelFlag::new()),
        Err(ArchiveError::TooManyEntries)
    );
    let (_temp, into) = staging();
    let big = Tar::default().file("big", &"x".repeat(5_000)).gz();
    let limits = ArchiveLimits {
        entries: MAX_ENTRIES,
        bytes: 4_000,
    };
    assert_eq!(
        unpack(big.as_slice(), &into, "", limits, &CancelFlag::new()),
        Err(ArchiveError::TooLarge)
    );
    // An extension entry the tar reader takes in whole (a long name) is
    // stopped by the cap on the decompressed stream.
    let (_temp, into) = staging();
    let long_name = Tar::default()
        .entry(
            "././@LongLink",
            b'L',
            &vec![b'a'; 3 * 1024 * 1024],
            "",
            0o644,
        )
        .entry(&format!("{TOP}/a"), b'0', b"x", "", 0o644)
        .raw_gz();
    let limits = ArchiveLimits {
        entries: 1,
        bytes: 10,
    };
    assert_eq!(
        unpack(long_name.as_slice(), &into, "", limits, &CancelFlag::new()),
        Err(ArchiveError::TooLarge)
    );
}

#[test]
fn unsafe_entries_refuse_the_archive() {
    let cases = [
        Tar::default()
            .entry("/etc/x", b'0', b"x", "", 0o644)
            .raw_gz(),
        Tar::default()
            .entry(&format!("{TOP}/a/../../x"), b'0', b"x", "", 0o644)
            .gz(),
        Tar::default().link("a", "/etc/passwd").gz(),
        Tar::default().link("a/b", "../../x").gz(),
        Tar::default()
            .entry(&format!("{TOP}/h"), b'1', &[], "README.md", 0o644)
            .gz(),
        Tar::default()
            .entry(&format!("{TOP}/c"), b'3', &[], "", 0o644)
            .gz(),
        Tar::default()
            .entry(&format!("{TOP}/b"), b'4', &[], "", 0o644)
            .gz(),
        Tar::default()
            .entry(&format!("{TOP}/p"), b'6', &[], "", 0o644)
            .gz(),
        Tar::default()
            .entry(&format!("{TOP}/\u{1}x"), b'0', b"x", "", 0o644)
            .gz(),
    ];
    for (at, archive) in cases.iter().enumerate() {
        assert_eq!(
            unpack_all(archive, "").1,
            Err(ArchiveError::Unsafe),
            "case {at}"
        );
    }
    // A link outside the link's folder is not looked at.
    let outside = Tar::default()
        .link("docs/x", "/etc/passwd")
        .skill("skills/a", "a", None)
        .gz();
    assert!(unpack_all(&outside, "skills/a").1.is_ok());
    // A link that stays in the repository is fine.
    let inside = Tar::default()
        .file("shared.md", "x")
        .link("skills/a/shared.md", "../../shared.md")
        .gz();
    assert!(unpack_all(&inside, "").1.is_ok());
}

#[test]
fn unreadable_archives_are_malformed_and_a_missing_folder_is_named() {
    assert_eq!(unpack_all(b"not gzip", "").1, Err(ArchiveError::Malformed));
    let two_tops = Tar::default()
        .entry("one/a", b'0', b"x", "", 0o644)
        .entry("two/b", b'0', b"x", "", 0o644)
        .raw_gz();
    assert_eq!(unpack_all(&two_tops, "").1, Err(ArchiveError::Malformed));
    let duplicate = Tar::default().file("a", "x").file("a", "y").gz();
    assert_eq!(unpack_all(&duplicate, "").1, Err(ArchiveError::Malformed));
    let link_over_file = Tar::default().file("a", "x").link("a", "b").gz();
    assert_eq!(
        unpack_all(&link_over_file, "").1,
        Err(ArchiveError::Malformed)
    );
    let file_at_top = Tar::default()
        .entry("loose", b'0', b"x", "", 0o644)
        .raw_gz();
    assert_eq!(unpack_all(&file_at_top, "").1, Err(ArchiveError::Malformed));
    let sparse = Tar::default()
        .entry(&format!("{TOP}/s"), b'S', &[], "", 0o644)
        .gz();
    assert_eq!(unpack_all(&sparse, "").1, Err(ArchiveError::Malformed));
    let empty = Tar::default().raw_gz();
    assert_eq!(unpack_all(&empty, "").1, Err(ArchiveError::Malformed));
    let archive = Tar::default().skill("skills/a", "a", None).gz();
    assert_eq!(
        unpack_all(&archive, "skills/b").1,
        Err(ArchiveError::PathNotFound)
    );
    // A prefix that is only part of a folder's name is not that folder.
    assert_eq!(
        unpack_all(&archive, "skills/").1,
        Err(ArchiveError::PathNotFound)
    );
    assert_eq!(
        unpack_all(&archive, "skill").1,
        Err(ArchiveError::PathNotFound)
    );
}

#[test]
fn a_cancelled_unpack_stops() {
    let (_temp, into) = staging();
    let cancel = CancelFlag::new();
    cancel.cancel();
    let archive = Tar::default().skill("a", "a", None).gz();
    assert_eq!(
        unpack(
            archive.as_slice(),
            &into,
            "",
            ArchiveLimits::default(),
            &cancel
        ),
        Err(ArchiveError::Cancelled)
    );
}

fn found_at(
    archive: &[u8],
    prefix: &str,
) -> (tempfile::TempDir, Unpacked, Result<Found, FindError>) {
    let (temp, unpacked) = unpack_all(archive, prefix);
    let unpacked = unpacked.unwrap();
    let found = find(&unpacked, prefix, "skills");
    (temp, unpacked, found)
}

fn paths(found: &Found) -> Vec<&str> {
    found
        .skills
        .iter()
        .map(|skill| skill.path.as_str())
        .collect()
}

#[test]
fn the_links_folder_is_the_skill_when_it_has_one() {
    let archive = Tar::default()
        .skill("skills/pdf", "pdf", Some("pdf"))
        .skill("skills/pdf/nested", "nested", None)
        .gz();
    let (_temp, _, found) = found_at(&archive, "skills/pdf");
    let found = found.unwrap();
    assert_eq!(paths(&found), ["skills/pdf"]);
    let pdf = &found.skills[0];
    assert_eq!(pdf.name, "pdf");
    assert_eq!(pdf.description, "The pdf skill.");
    assert_eq!(pdf.folder_name.as_deref(), Some("pdf"));
    assert!(matches!(&pdf.rubric, RubricStatus::Ready(rubric) if rubric.id == "pdf"));
    assert_eq!(pdf.files, 3);
    assert_eq!(
        pdf.bytes,
        (skill_md("pdf", "The pdf skill.").len()
            + rubric_text("pdf").len()
            + skill_md("nested", "The nested skill.").len()) as u64
    );
    assert_eq!(pdf.problem, None);
}

#[test]
fn skills_under_the_folder_are_found_up_to_three_levels_by_path() {
    let archive = Tar::default()
        .skill("skills/b", "b", None)
        .skill("skills/a", "a", Some("a"))
        .skill("skills/a/inner", "inner", None)
        .skill("deep/one/two", "two", None)
        .skill("deep/one/two/three/four", "four", None)
        .skill("too/deep/for/it", "far", None)
        .skill(".claude/skills/hidden", "hidden", None)
        .skill(".git/hooks/x", "git", None)
        .file("README.md", "x")
        .gz();
    let (_temp, _, found) = found_at(&archive, "");
    let found = found.unwrap();
    assert_eq!(
        paths(&found),
        [
            ".claude/skills/hidden",
            "deep/one/two",
            "skills/a",
            "skills/b"
        ]
    );
    assert!(!found.incomplete);
    let (_temp, _, found) = found_at(&archive, "skills");
    assert_eq!(paths(&found.unwrap()), ["skills/a", "skills/b"]);
}

#[test]
fn no_skill_and_a_file_link_are_named() {
    let archive = Tar::default()
        .file("README.md", "x")
        .file("docs/a/b/c/SKILL.md", "deep")
        .gz();
    let (_temp, _, found) = found_at(&archive, "");
    assert_eq!(found, Err(FindError::NoSkill));
    let (_temp, unpacked) = unpack_all(&archive, "README.md");
    assert_eq!(
        find(&unpacked.unwrap(), "README.md", "skills"),
        Err(FindError::PathNotFound)
    );
}

#[test]
fn more_skills_than_listed_says_so() {
    let archive = (0..=MAX_FOUND)
        .fold(Tar::default(), |tar, at| {
            tar.skill(&format!("s/{at:03}"), "x", None)
        })
        .gz();
    let (_temp, _, found) = found_at(&archive, "");
    let found = found.unwrap();
    assert_eq!(found.skills.len(), MAX_FOUND);
    assert!(found.incomplete);
    assert_eq!(found.skills[0].path, "s/000");
}

#[test]
fn each_skill_says_its_rubric_and_what_keeps_it_from_being_kept() {
    let archive = Tar::default()
        .skill("s/ready", "ready", Some("ready"))
        .skill("s/broken", "broken", None)
        .file("s/broken/xtrace-rubric.md", "no header")
        .skill("s/linked-rubric", "linked-rubric", None)
        .file("s/linked-rubric/rubric.txt", &rubric_text("x"))
        .link("s/linked-rubric/xtrace-rubric.md", "rubric.txt")
        .skill("s/link-out", "link-out", None)
        .link("s/link-out/up", "../ready/SKILL.md")
        .skill("s/link-folder", "link-folder", None)
        .dir("s/link-folder/sub")
        .link("s/link-folder/to-sub", "sub")
        .skill("s/link-nothing", "link-nothing", None)
        .link("s/link-nothing/gone", "missing.md")
        .link("s/link-md/SKILL.md", "../ready/SKILL.md")
        .entry(
            &format!("{TOP}/s/latin1/SKILL.md"),
            b'0',
            b"---\nname: \xe9\n---\n",
            "",
            0o644,
        )
        .file("s/no-front/SKILL.md", "Just text.\n")
        .gz();
    let (_temp, _, found) = found_at(&archive, "s");
    let found = found.unwrap();
    let by = |path: &str| {
        found
            .skills
            .iter()
            .find(|skill| skill.path == format!("s/{path}"))
            .unwrap()
    };
    assert!(matches!(by("ready").rubric, RubricStatus::Ready(_)));
    assert_eq!(by("ready").problem, None);
    assert_eq!(
        by("broken").rubric,
        RubricStatus::Problem(RubricError::NoHeader)
    );
    assert_eq!(
        by("linked-rubric").rubric,
        RubricStatus::Problem(RubricError::NotRegularFile)
    );
    assert_eq!(by("linked-rubric").problem, None);
    assert_eq!(by("link-out").problem, Some(SkillProblem::LinkLeavesFolder));
    assert_eq!(
        by("link-folder").problem,
        Some(SkillProblem::LinkLeavesFolder)
    );
    assert_eq!(
        by("link-nothing").problem,
        Some(SkillProblem::LinkLeavesFolder)
    );
    assert_eq!(by("link-md").problem, Some(SkillProblem::LinkLeavesFolder));
    assert_eq!(
        by("latin1").problem,
        Some(SkillProblem::InstructionsUnreadable)
    );
    assert_eq!(by("latin1").name, "latin1");
    assert_eq!(by("no-front").problem, None);
    assert_eq!(by("no-front").name, "no-front");
    assert_eq!(by("no-front").description, "");
    assert_eq!(by("no-front").rubric, RubricStatus::Missing);
}

#[test]
fn a_skill_past_the_installed_copy_limits_cannot_be_kept() {
    let archive = (0..MAX_INSTALLED_SKILL_FILES)
        .fold(Tar::default().skill("big", "big", None), |tar, at| {
            tar.file(&format!("big/f{at}"), "x")
        })
        .gz();
    let (_temp, _, found) = found_at(&archive, "big");
    assert_eq!(
        found.unwrap().skills[0].problem,
        Some(SkillProblem::TooLarge)
    );
}

#[test]
fn the_folder_name_is_the_skill_name_else_its_own_folders() {
    assert_eq!(
        folder_name(Some("pdf-tools"), "x").as_deref(),
        Some("pdf-tools")
    );
    assert_eq!(folder_name(Some("a1"), "x").as_deref(), Some("a1"));
    for odd in ["PDF", "pdf tools", "-pdf", "pdf-", "pdf--x", "pdf_x", ""] {
        assert_eq!(
            folder_name(Some(odd), "own").as_deref(),
            Some("own"),
            "{odd}"
        );
    }
    assert_eq!(
        folder_name(Some(&"a".repeat(65)), "own").as_deref(),
        Some("own")
    );
    assert_eq!(folder_name(None, "My Skill").as_deref(), Some("My Skill"));
    assert_eq!(folder_name(None, ".hidden"), None);
    assert_eq!(folder_name(None, "a\u{7}b"), None);
    assert_eq!(folder_name(None, &"a".repeat(101)), None);
}

fn found_skill(path: &str, folder: Option<&str>) -> FoundSkill {
    FoundSkill {
        path: path.into(),
        name: path.into(),
        description: String::new(),
        files: 1,
        bytes: 1,
        rubric: RubricStatus::Missing,
        folder_name: folder.map(str::to_owned),
        problem: None,
    }
}

#[test]
fn the_keep_rule_names_each_clash_in_order() {
    let home = tempfile::tempdir().unwrap();
    let agents = home.path().join(AGENTS_SKILLS);
    let claude = home.path().join(CLAUDE_SKILLS);
    std::fs::create_dir_all(agents.join("taken")).unwrap();
    std::fs::create_dir_all(&claude).unwrap();
    std::os::unix::fs::symlink("/nowhere", claude.join("dangling")).unwrap();
    std::fs::write(claude.join("claude-only"), "x").unwrap();
    let mut too_large = found_skill("big", Some("big"));
    too_large.problem = Some(SkillProblem::TooLarge);
    let skills = [
        found_skill("a/taken", Some("taken")),
        found_skill("a/dangling", Some("dangling")),
        found_skill("a/claude-only", Some("claude-only")),
        found_skill("a/free", Some("free")),
        found_skill("b/free", Some("Free")),
        found_skill("odd", None),
        too_large,
        found_skill("c/free", Some("free")),
    ];
    assert_eq!(
        keep_states(&skills, home.path()),
        vec![
            KeepState::NameInAgents(agents.join("taken")),
            KeepState::NameInClaude(claude.join("dangling")),
            KeepState::NameInClaude(claude.join("claude-only")),
            KeepState::Keepable,
            KeepState::SameNameAbove("a/free".into()),
            KeepState::UnusableName,
            KeepState::Problem(SkillProblem::TooLarge),
            KeepState::SameNameAbove("a/free".into()),
        ]
    );
}

fn kept_archive() -> Vec<u8> {
    Tar::default()
        .skill("skills/pdf", "pdf", Some("pdf"))
        .entry(
            &format!("{TOP}/skills/pdf/scripts/run.sh"),
            b'0',
            b"#!/bin/sh\n",
            "",
            0o755,
        )
        .file("skills/pdf/.hidden", "kept too")
        .link("skills/pdf/alias.md", "SKILL.md")
        .gz()
}

#[test]
fn a_keep_told_to_stop_before_the_claim_leaves_no_copy_and_no_claimed_name() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    let (_temp, unpacked, found) = found_at(&kept_archive(), "skills/pdf");
    let skill = &found.unwrap().skills[0];
    let agents = agents_folder(home.path()).unwrap();
    let left = || std::fs::read_dir(&agents).unwrap().count();
    // Told before copying, and told after copying (just before the claim).
    for stop_at in [1, 2] {
        let asked = std::cell::Cell::new(0);
        let kept = keep_unless(&unpacked, skill, home.path(), &agents, "t", || {
            asked.set(asked.get() + 1);
            asked.get() >= stop_at
        });
        assert_eq!(kept, Kept::Failed, "stop at {stop_at}");
        assert_eq!(asked.get(), stop_at);
        assert_eq!(left(), 0, "stop at {stop_at}");
        assert!(!home.path().join(".claude/skills/pdf").exists());
    }
    // The name is still free: keeping it again works.
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept { .. }
    ));
}

#[test]
fn keeping_copies_the_skill_in_place_and_links_it_for_claude() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    let (_temp, unpacked, found) = found_at(&kept_archive(), "skills/pdf");
    let skill = &found.unwrap().skills[0];
    let agents = agents_folder(home.path()).unwrap();
    let kept = keep(&unpacked, skill, home.path(), &agents, "t1");
    let folder = home.path().join(".agents/skills/pdf");
    let link = home.path().join(".claude/skills/pdf");
    assert_eq!(
        kept,
        Kept::Kept {
            folder: folder.clone(),
            claude: ClaudeLink::Linked(link.clone()),
        }
    );
    use std::os::unix::fs::PermissionsExt;
    assert!(folder.join("SKILL.md").is_file());
    assert!(folder.join("xtrace-rubric.md").is_file());
    assert_eq!(
        std::fs::read_to_string(folder.join(".hidden")).unwrap(),
        "kept too"
    );
    assert_eq!(
        std::fs::metadata(folder.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert_eq!(
        std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(
        std::fs::read_link(folder.join("alias.md")).unwrap(),
        PathBuf::from("SKILL.md")
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), folder);
    // No hidden copy is left behind.
    let names: Vec<_> = std::fs::read_dir(&agents)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["pdf"]);
    // Installed discovery now lists it once, from ~/.agents/skills, ready.
    let discovered = crate::installed::discover(home.path());
    assert_eq!(discovered.skills.len(), 1);
    assert_eq!(discovered.skills[0].folder, folder);
    assert_eq!(discovered.skipped, 0);
    assert!(matches!(
        discovered.skills[0].rubric,
        RubricStatus::Ready(_)
    ));
    // Keeping it again never overwrites it.
    std::fs::write(folder.join("SKILL.md"), "mine").unwrap();
    assert_eq!(
        keep(&unpacked, skill, home.path(), &agents, "t2"),
        Kept::NameTaken
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("SKILL.md")).unwrap(),
        "mine"
    );
    let names: Vec<_> = std::fs::read_dir(&agents)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["pdf"]);
}

#[test]
fn the_claude_link_is_skipped_when_taken_or_without_claude() {
    let (_temp, unpacked, found) = found_at(&kept_archive(), "skills/pdf");
    let skill = &found.unwrap().skills[0];
    // No ~/.claude at all: nothing is made there.
    let home = tempfile::tempdir().unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::NoClaudeFolder,
            ..
        }
    ));
    assert!(!home.path().join(".claude").exists());
    // ~/.claude/skills/pdf taken (by a file): it stays.
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(CLAUDE_SKILLS)).unwrap();
    std::fs::write(home.path().join(".claude/skills/pdf"), "theirs").unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::NameTaken,
            ..
        }
    ));
    assert_eq!(
        std::fs::read_to_string(home.path().join(".claude/skills/pdf")).unwrap(),
        "theirs"
    );
    // ~/.claude/skills is a file: no link.
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    std::fs::write(home.path().join(CLAUDE_SKILLS), "x").unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::Failed,
            ..
        }
    ));
}

#[test]
fn the_agents_folder_must_be_a_real_folder() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".agents")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), home.path().join(AGENTS_SKILLS)).unwrap();
    assert_eq!(agents_folder(home.path()), Err(KeepError::AgentsFolder));
    let home = tempfile::tempdir().unwrap();
    let made = agents_folder(home.path()).unwrap();
    assert!(made.is_dir());
}

#[test]
fn a_copy_that_fails_leaves_nothing_behind() {
    let home = tempfile::tempdir().unwrap();
    let (_temp, unpacked, found) = found_at(&kept_archive(), "skills/pdf");
    let mut skill = found.unwrap().skills[0].clone();
    let agents = agents_folder(home.path()).unwrap();
    // A staged folder that is gone cannot be copied.
    skill.path = "skills/gone".into();
    assert_eq!(
        keep(&unpacked, &skill, home.path(), &agents, "t"),
        Kept::Failed
    );
    assert_eq!(std::fs::read_dir(&agents).unwrap().count(), 0);
}

fn record(folder: &str) -> LinkedSkill {
    LinkedSkill {
        folder: folder.into(),
        claude_link: None,
        repository: "octo/skills".into(),
        path: "skills/pdf".into(),
        reference: Some("main".into()),
        commit: "0123456789abcdef0123456789abcdef01234567".into(),
        fetched_at_ms: 1,
        kept_at_ms: 2,
    }
}

#[test]
fn records_are_kept_beside_xtraces_data_and_replaced_by_folder() {
    let data = tempfile::tempdir().unwrap();
    let file = LinkedSkills::new(data.path().join(LINKED_SKILLS_FILE));
    assert!(file.read().is_empty());
    let mut again = record("/a");
    again.commit = "f".repeat(40);
    let all = file
        .update(|skills| {
            recorded(skills, record("/a"))
                && recorded(skills, record("/b"))
                && recorded(skills, again.clone())
        })
        .unwrap();
    assert!(all);
    assert_eq!(file.read(), vec![record("/b"), again]);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Full: nothing more is recorded.
    let mut full: Vec<LinkedSkill> = (0..MAX_RECORDS)
        .map(|at| record(&format!("/{at}")))
        .collect();
    assert!(!recorded(&mut full, record("/new")));
    assert_eq!(full.len(), MAX_RECORDS);
    assert!(recorded(&mut full, record("/0")));
}

#[test]
fn an_unusable_records_file_is_moved_aside_never_overwritten() {
    let data = tempfile::tempdir().unwrap();
    let file = LinkedSkills::new(data.path().join(LINKED_SKILLS_FILE));
    let bad = file.set_aside_path();
    assert_eq!(bad, data.path().join("linked-skills.json.bad"));
    for (at, text) in ["not json", r#"{"layout":2,"skills":[]}"#]
        .into_iter()
        .enumerate()
    {
        std::fs::write(file.path(), text).unwrap();
        // Another layout, or not JSON, reads as none.
        assert!(file.read().is_empty());
        let folder = format!("/{at}");
        file.update(|skills| recorded(skills, record(&folder)))
            .unwrap();
        // The old bytes are kept beside it (replacing an older .bad).
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), text);
        assert_eq!(file.read(), vec![record(&folder)]);
    }
    // A file that cannot be moved aside is not overwritten either.
    std::fs::remove_file(file.path()).unwrap();
    std::fs::remove_file(&bad).unwrap();
    std::fs::create_dir(file.path()).unwrap();
    std::fs::create_dir(&bad).unwrap();
    std::fs::write(bad.join("keep"), "x").unwrap();
    assert!(
        file.update(|skills| recorded(skills, record("/x")))
            .is_err()
    );
    assert!(file.path().is_dir());
}

#[test]
fn records_written_at_once_are_all_kept() {
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join(LINKED_SKILLS_FILE);
    let threads: Vec<_> = (0..8)
        .map(|at| {
            let file = LinkedSkills::new(path.clone());
            std::thread::spawn(move || {
                file.update(|skills| recorded(skills, record(&format!("/{at}"))))
                    .unwrap()
            })
        })
        .collect();
    for thread in threads {
        assert!(thread.join().unwrap());
    }
    let mut folders: Vec<String> = LinkedSkills::new(path)
        .read()
        .into_iter()
        .map(|skill| skill.folder)
        .collect();
    folders.sort();
    assert_eq!(
        folders,
        (0..8).map(|at| format!("/{at}")).collect::<Vec<_>>()
    );
    // No temporary file is left.
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 1);
}

#[test]
fn leftover_copies_are_removed_but_nothing_else() {
    let home = tempfile::tempdir().unwrap();
    let agents = home.path().join(AGENTS_SKILLS);
    std::fs::create_dir_all(agents.join(".xtrace-keeping-abc-0/sub")).unwrap();
    std::fs::write(agents.join(".xtrace-keeping-abc-0/sub/f"), "x").unwrap();
    std::fs::create_dir(agents.join(".xtrace-keepingX")).unwrap();
    std::fs::write(agents.join(".xtrace-keeping-file"), "x").unwrap();
    std::fs::create_dir(agents.join("pdf")).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("f"), "x").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), agents.join(".xtrace-keeping-link")).unwrap();
    assert_eq!(remove_keeping_leftovers(home.path()), 1);
    let mut names: Vec<_> = std::fs::read_dir(&agents)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            ".xtrace-keeping-file",
            ".xtrace-keeping-link",
            ".xtrace-keepingX",
            "pdf"
        ]
    );
    assert!(elsewhere.path().join("f").exists());
    // No ~/.agents/skills: nothing to do.
    assert_eq!(remove_keeping_leftovers(elsewhere.path()), 0);
}

/// Links are read from inside the skill's own folder, as they will be once
/// it is kept (possibly under another name): climbing out and back in by
/// the repository's folder names is out.
#[test]
fn links_are_read_from_the_skills_own_folder() {
    let archive = Tar::default()
        // Climbs out of plugins/foo and back in by the repository's names.
        .skill("plugins/foo", "foo", None)
        .file("plugins/foo/secret.txt", "x")
        .link("plugins/foo/x", "../../plugins/foo/secret.txt")
        // Kept as "bar", so `../foo/f` would name another folder then.
        .skill("skills/foo", "bar", None)
        .file("skills/foo/f", "x")
        .link("skills/foo/a", "../foo/f")
        // A link to a link.
        .skill("skills/chain", "chain", None)
        .file("skills/chain/f", "x")
        .link("skills/chain/b", "f")
        .link("skills/chain/a", "b")
        // A link to a folder inside it.
        .skill("skills/to-folder", "to-folder", None)
        .file("skills/to-folder/sub/f", "x")
        .link("skills/to-folder/s", "sub")
        // Links that stay inside, from a subfolder and down into one.
        .skill("skills/fine", "fine", None)
        .file("skills/fine/sub/f", "x")
        .link("skills/fine/sub/up", "../SKILL.md")
        .link("skills/fine/down", "./sub/f")
        .gz();
    let (_temp, unpacked) = unpack_all(&archive, "");
    let found = find(&unpacked.unwrap(), "", "skills").unwrap();
    let problem = |path: &str| {
        found
            .skills
            .iter()
            .find(|skill| skill.path == path)
            .unwrap()
            .problem
    };
    for path in [
        "plugins/foo",
        "skills/foo",
        "skills/chain",
        "skills/to-folder",
    ] {
        assert_eq!(
            problem(path),
            Some(SkillProblem::LinkLeavesFolder),
            "{path}"
        );
    }
    assert_eq!(problem("skills/fine"), None);
    // The same skill linked on its own is read the same way.
    let (_temp, unpacked) = unpack_all(&archive, "plugins/foo");
    let found = find(&unpacked.unwrap(), "plugins/foo", "skills").unwrap();
    assert_eq!(
        found.skills[0].problem,
        Some(SkillProblem::LinkLeavesFolder)
    );
}

#[test]
fn a_linked_claude_folder_the_user_owns_is_followed() {
    let (_temp, unpacked, found) = found_at(&kept_archive(), "skills/pdf");
    let skill = &found.unwrap().skills[0];
    // ~/.claude is a link to a folder of the user's.
    let home = tempfile::tempdir().unwrap();
    let real = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(real.path(), home.path().join(".claude")).unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert_eq!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            folder: agents.join("pdf"),
            claude: ClaudeLink::Linked(home.path().join(".claude/skills/pdf")),
        }
    );
    assert_eq!(
        std::fs::read_link(real.path().join("skills/pdf")).unwrap(),
        agents.join("pdf")
    );
    // ~/.claude/skills is a link to a folder of the user's.
    let home = tempfile::tempdir().unwrap();
    let real = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink(real.path(), home.path().join(CLAUDE_SKILLS)).unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::Linked(_),
            ..
        }
    ));
    assert!(std::fs::symlink_metadata(real.path().join("pdf")).is_ok());
    // A link to nothing is no ~/.claude; a link to a folder someone else
    // owns (the file system's root) is not used.
    let home = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/nowhere/at/all", home.path().join(".claude")).unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::NoClaudeFolder,
            ..
        }
    ));
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    std::os::unix::fs::symlink("/", home.path().join(CLAUDE_SKILLS)).unwrap();
    let agents = agents_folder(home.path()).unwrap();
    assert!(matches!(
        keep(&unpacked, skill, home.path(), &agents, "t"),
        Kept::Kept {
            claude: ClaudeLink::Failed,
            ..
        }
    ));
}
