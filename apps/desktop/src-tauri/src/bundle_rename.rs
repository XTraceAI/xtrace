//! The macOS app was named "XTrace Desktop" through release 0.1.7 and is
//! named "XTrace" since. An update writes the new version over the bundle
//! folder it replaces, so a copy installed as `XTrace Desktop.app` keeps that
//! folder name after updating, and Finder and Spotlight keep showing it. On a
//! normal quit that copy renames its own folder after the current
//! `productName`, `XTrace.app`.
//!
//! [`plan`] is the one rule for whether a quit renames the bundle, and to
//! what. Nothing is ever deleted or replaced: when anything is already at
//! `XTrace.app`, both stay as they are.
use crate::{earlier_install, telemetry::INSTALLED_IDENTIFIER};
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

/// The bundle folder name releases through 0.1.7 installed under.
pub const EARLIER_BUNDLE: &str = "XTrace Desktop.app";

/// One rename of the bundle folder, in the folder it is in.
#[derive(Debug, PartialEq, Eq)]
pub struct Rename {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Whether this exit renames the running bundle, and to what.
///
/// The new name is `<product_name>.app`. Only the installed app's
/// identifier, only a bundle folder named exactly [`EARLIER_BUNDLE`], never
/// on a restart (Tauri relaunches from the path
/// the app is at), and never when `taken` says something is already at the
/// new name. The bundle folder is the one the updater replaces, worked out
/// from the executable by the updater's own rule (`exe` → `Contents/MacOS`
/// → `.app`); an executable outside `Contents/MacOS` (`cargo run`) has none.
pub fn plan(
    executable: &Path,
    identifier: &str,
    product_name: Option<&str>,
    restarting: bool,
    taken: impl FnOnce(&Path) -> bool,
) -> Option<Rename> {
    // The updater would take any other executable's own folder as the
    // bundle; only one inside `Contents/MacOS` is in a bundle.
    let in_bundle = executable
        .parent()
        .is_some_and(|folder| folder.ends_with("Contents/MacOS"));
    if restarting || identifier != INSTALLED_IDENTIFIER || !in_bundle {
        return None;
    }
    let from = tauri_plugin_updater::extract_path_from_executable(executable).ok()?;
    if from.file_name() != Some(OsStr::new(EARLIER_BUNDLE)) {
        return None;
    }
    let to = from.with_file_name(format!("{}.app", product_name?));
    if to == from || taken(&to) {
        return None;
    }
    Some(Rename { from, to })
}

impl Rename {
    /// One atomic rename that fails, changing nothing, when anything has
    /// appeared at the new name since [`plan`] looked.
    pub fn apply(&self) -> io::Result<()> {
        rename_exclusive(&self.from, &self.to)
    }
}

/// Renames the running bundle when [`plan`] says so. Runs on a normal quit,
/// after the services and the database have stopped. If it cannot rename
/// (for example an account that cannot change Applications), nothing changes
/// and the app keeps working under the old name; quitting is never blocked.
/// The error goes to standard error, which an app opened from Finder does
/// not keep.
pub fn rename_on_quit(config: &tauri::Config, restarting: bool) {
    let Ok(executable) = tauri::utils::platform::current_exe() else {
        return;
    };
    let Some(rename) = plan(
        &executable,
        &config.identifier,
        config.product_name.as_deref(),
        restarting,
        |to| !earlier_install::absent(to),
    ) else {
        return;
    };
    if let Err(error) = rename.apply() {
        eprintln!(
            "XTrace could not rename {} to {}: {error}",
            rename.from.display(),
            rename.to.display()
        );
    }
}

#[cfg(target_os = "macos")]
fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let path = |path: &Path| {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
    };
    let (from, to) = (path(from)?, path(to)?);
    // SAFETY: both are valid NUL-terminated paths that outlive the call.
    // RENAME_EXCL makes the rename fail with EEXIST instead of replacing
    // whatever is at `to`, which plain rename(2) would do to an empty folder.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn rename_exclusive(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

// The bundle folder is worked out by the updater's macOS rule.
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;

    const EARLIER_EXE: &str = "/Applications/XTrace Desktop.app/Contents/MacOS/xtrace-desktop";
    const BUNDLE: &str = "XTrace.app";
    const PRODUCT: Option<&str> = Some("XTrace");

    fn free(_: &Path) -> bool {
        false
    }

    #[test]
    fn earlier_name_with_the_installed_id_renames_beside_itself() {
        assert_eq!(
            plan(
                Path::new(EARLIER_EXE),
                "ai.xtrace.app",
                PRODUCT,
                false,
                free
            ),
            Some(Rename {
                from: PathBuf::from("/Applications/XTrace Desktop.app"),
                to: PathBuf::from("/Applications/XTrace.app"),
            })
        );
        // In another folder, it stays in that folder.
        assert_eq!(
            plan(
                Path::new(
                    "/Users/someone/Applications/XTrace Desktop.app/Contents/MacOS/xtrace-desktop"
                ),
                "ai.xtrace.app",
                PRODUCT,
                false,
                free
            )
            .map(|rename| rename.to),
            Some(PathBuf::from("/Users/someone/Applications/XTrace.app"))
        );
    }

    #[test]
    fn a_bundle_already_named_xtrace_is_left_alone() {
        let exe = "/Applications/XTrace.app/Contents/MacOS/xtrace-desktop";
        assert_eq!(
            plan(Path::new(exe), "ai.xtrace.app", PRODUCT, false, free),
            None
        );
    }

    #[test]
    fn the_shipped_config_renames_to_xtrace_app() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let identifier = config["identifier"].as_str().unwrap();
        let product = config["productName"].as_str();
        assert_eq!(
            plan(Path::new(EARLIER_EXE), identifier, product, false, free).map(|r| r.to),
            Some(PathBuf::from("/Applications/XTrace.app"))
        );
        // Without a product name there is no new name.
        assert_eq!(
            plan(Path::new(EARLIER_EXE), identifier, None, false, free),
            None
        );
    }

    #[test]
    fn other_names_are_left_alone() {
        for exe in [
            "/Applications/XTrace Desktop 2.app/Contents/MacOS/xtrace-desktop",
            "/Applications/xtrace desktop.app/Contents/MacOS/xtrace-desktop",
            "/Applications/XTrace Desktop/Contents/MacOS/xtrace-desktop",
        ] {
            assert_eq!(
                plan(Path::new(exe), "ai.xtrace.app", PRODUCT, false, free),
                None,
                "{exe}"
            );
        }
    }

    #[test]
    fn test_copies_and_other_ids_are_left_alone() {
        for id in [
            "ai.xtrace.app.test",
            "ai.xtrace.app.test.abc123",
            "ai.xtrace.desktop",
            "",
        ] {
            assert_eq!(
                plan(Path::new(EARLIER_EXE), id, PRODUCT, false, free),
                None,
                "{id}"
            );
        }
    }

    #[test]
    fn anything_at_the_new_name_keeps_both() {
        let mut asked = None;
        let taken = |to: &Path| {
            asked = Some(to.to_path_buf());
            true
        };
        assert_eq!(
            plan(
                Path::new(EARLIER_EXE),
                "ai.xtrace.app",
                PRODUCT,
                false,
                taken
            ),
            None
        );
        assert_eq!(asked, Some(PathBuf::from("/Applications/XTrace.app")));
    }

    #[test]
    fn a_restart_is_never_renamed() {
        assert_eq!(
            plan(Path::new(EARLIER_EXE), "ai.xtrace.app", PRODUCT, true, free),
            None
        );
    }

    #[test]
    fn an_executable_outside_a_bundle_is_left_alone() {
        for exe in [
            "/Users/someone/repo/target/debug/xtrace-desktop",
            "/Users/someone/XTrace Desktop.app/xtrace-desktop",
            "/Users/someone/XTrace Desktop.app/Contents/MacOS/tools/xtrace-desktop",
            "xtrace-desktop",
        ] {
            assert_eq!(
                plan(Path::new(exe), "ai.xtrace.app", PRODUCT, false, free),
                None,
                "{exe}"
            );
        }
    }

    /// A fake bundle folder with an executable and one resource.
    fn bundle(parent: &Path, name: &str) -> PathBuf {
        let bundle = parent.join(name);
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
        fs::write(bundle.join("Contents/MacOS/xtrace-desktop"), b"exe").unwrap();
        fs::write(bundle.join("Contents/Resources/resource"), name).unwrap();
        bundle
    }

    fn plan_on_disk(exe: &Path) -> Option<Rename> {
        plan(exe, "ai.xtrace.app", PRODUCT, false, |to| {
            !earlier_install::absent(to)
        })
    }

    #[test]
    fn renames_a_real_folder_and_keeps_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = bundle(dir.path(), EARLIER_BUNDLE);
        let rename = plan_on_disk(&earlier.join("Contents/MacOS/xtrace-desktop")).unwrap();
        rename.apply().unwrap();

        assert!(earlier_install::absent(&earlier));
        let renamed = dir.path().join(BUNDLE);
        assert_eq!(
            fs::read(renamed.join("Contents/MacOS/xtrace-desktop")).unwrap(),
            b"exe"
        );
        assert_eq!(
            fs::read_to_string(renamed.join("Contents/Resources/resource")).unwrap(),
            EARLIER_BUNDLE
        );
        // Once renamed, it is not renamed again.
        assert_eq!(
            plan_on_disk(&renamed.join("Contents/MacOS/xtrace-desktop")),
            None
        );
    }

    #[test]
    fn an_existing_xtrace_app_keeps_both_folders_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = bundle(dir.path(), EARLIER_BUNDLE);
        let current = bundle(dir.path(), BUNDLE);
        assert_eq!(
            plan_on_disk(&earlier.join("Contents/MacOS/xtrace-desktop")),
            None
        );
        for (folder, name) in [(&earlier, EARLIER_BUNDLE), (&current, BUNDLE)] {
            assert_eq!(
                fs::read_to_string(folder.join("Contents/Resources/resource")).unwrap(),
                name
            );
        }
    }

    #[test]
    fn something_appearing_after_the_check_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = bundle(dir.path(), EARLIER_BUNDLE);
        let rename = plan_on_disk(&earlier.join("Contents/MacOS/xtrace-desktop")).unwrap();
        // An empty folder is what a plain rename would silently replace; a
        // link is refused too.
        let target = dir.path().join(BUNDLE);
        fs::create_dir(&target).unwrap();
        let error = rename.apply().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(earlier.join("Contents/MacOS/xtrace-desktop").is_file());
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);

        fs::remove_dir(&target).unwrap();
        std::os::unix::fs::symlink(dir.path().join("missing"), &target).unwrap();
        assert_eq!(
            rename.apply().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(earlier.join("Contents/MacOS/xtrace-desktop").is_file());
        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn a_folder_without_write_permission_keeps_its_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("Applications");
        let earlier = bundle(&parent, EARLIER_BUNDLE);
        let rename = plan_on_disk(&earlier.join("Contents/MacOS/xtrace-desktop")).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
        let result = rename.apply();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert!(earlier.join("Contents/MacOS/xtrace-desktop").is_file());
        assert!(earlier_install::absent(&parent.join(BUNDLE)));
    }

    #[test]
    fn a_failed_rename_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let earlier = bundle(dir.path(), EARLIER_BUNDLE);
        let rename = plan_on_disk(&earlier.join("Contents/MacOS/xtrace-desktop")).unwrap();
        fs::remove_dir_all(&earlier).unwrap();
        assert_eq!(rename.apply().unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(earlier_install::absent(&dir.path().join(BUNDLE)));
    }
}
