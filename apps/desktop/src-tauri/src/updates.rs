use crate::update_config;
use std::sync::atomic::{AtomicBool, Ordering};

fn enabled(debug: bool, fixtures: bool, macos: bool, configured: bool) -> bool {
    !debug && !fixtures && macos && configured
}

fn configured<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    let config = app.config().plugins.0.get("updater");
    update_config::validate(config) == Ok(true)
        && config.is_some_and(|value| {
            serde_json::from_value::<tauri_plugin_updater::Config>(value.clone()).is_ok()
        })
}

#[tauri::command]
pub fn updates_enabled<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> bool {
    enabled(
        cfg!(debug_assertions),
        cfg!(feature = "fixtures"),
        cfg!(target_os = "macos"),
        configured(&app),
    )
}

pub fn setup(app: &tauri::AppHandle) -> tauri::Result<()> {
    if updates_enabled(app.clone()) {
        app.plugin(tauri_plugin_updater::Builder::new().build())?;
    }
    Ok(())
}

#[tauri::command]
pub fn restart_after_update(app: tauri::AppHandle) -> Result<(), &'static str> {
    if !updates_enabled(app.clone()) {
        return Err("updates are disabled in this build");
    }
    // Recorded before the request, so a quit that arrives in the same instant
    // still ends as the restart Tauri performs after RunEvent::Exit.
    RESTART.request();
    // Unlike restart() on the main thread, this requests the normal Exit path.
    // Existing worker and database cleanup stays in the app's RunEvent::Exit.
    app.request_restart();
    Ok(())
}

/// Whether this run of the app ends in a restart. Set by
/// [`restart_after_update`] before it asks Tauri to restart, or by an exit
/// request carrying [`tauri::RESTART_EXIT_CODE`] (Tauri's own restart
/// request, which it never lets be prevented). Once set it stays set: after
/// `RunEvent::Exit` Tauri relaunches the app from the path it started at.
pub struct RestartFlag(AtomicBool);

impl RestartFlag {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn exit_requested(&self, exit_code: Option<i32>) {
        if exit_code == Some(tauri::RESTART_EXIT_CODE) {
            self.request();
        }
    }

    fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

static RESTART: RestartFlag = RestartFlag::new();

/// Called for every `RunEvent::ExitRequested`.
pub fn exit_requested(exit_code: Option<i32>) {
    RESTART.exit_requested(exit_code);
}

/// The one answer to "is this exit a restart?", read in `RunEvent::Exit`.
pub fn restarting() -> bool {
    RESTART.is_set()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_request_or_tauris_restart_code_marks_the_exit_a_restart() {
        let flag = RestartFlag::new();
        assert!(!flag.is_set());
        // A plain quit is not a restart.
        flag.exit_requested(None);
        flag.exit_requested(Some(0));
        assert!(!flag.is_set());
        // Restart to update sets it before any exit request is seen, so a
        // quit request that arrives first still ends as a restart.
        flag.request();
        flag.exit_requested(Some(0));
        assert!(flag.is_set());

        let flag = RestartFlag::new();
        flag.exit_requested(Some(tauri::RESTART_EXIT_CODE));
        assert!(flag.is_set());
        flag.exit_requested(None);
        assert!(flag.is_set());
    }

    #[test]
    fn updates_require_configured_supported_release() {
        assert!(enabled(false, false, true, true));
        assert!(!enabled(true, false, true, true));
        assert!(!enabled(false, true, true, true));
        assert!(!enabled(false, false, false, true));
        assert!(!enabled(false, false, true, false));
    }

    #[test]
    fn public_overlay_and_packaged_local_modes_select_opposite_controls() {
        for (debug, configured, public) in [
            (true, false, false),
            (true, true, false),
            (false, false, false),
            (false, true, true),
        ] {
            let updates = enabled(debug, false, true, configured);
            assert_eq!(updates, public);
            assert_eq!(
                crate::local_updates::require_local_updates(false, updates).is_ok(),
                !public
            );
        }
    }

    #[test]
    fn conflicting_signed_version_alias_cannot_enable_updates() {
        let config = serde_json::json!({"pubkey":"unused", "endpoints":["https://updates.example/latest.json"], "requireSignedVersion":true, "require-signed-version":false});
        assert!(serde_json::from_value::<tauri_plugin_updater::Config>(config).is_err());
    }
}
