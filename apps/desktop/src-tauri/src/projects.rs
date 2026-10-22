//! The user's projects, as the coding apps' own saved files list them.
//!
//! The source is chosen once, from the native home startup already selected
//! and validated. Constructing the service touches nothing on disk, and
//! nothing reads on a timer, at startup, on focus or on an event: only an
//! explicit `projects_list` reads. Fixture mode has no native home and so
//! nothing to read: it answers `not_configured`, and never falls back to the
//! user's own home.
//!
//! A read opens a handful of small files for reading only and holds no
//! database lock. Nothing is persisted, cached or logged; folder paths reach
//! the local view and nowhere else.
use crate::projects_dto::ProjectList;
use std::path::{Path, PathBuf};
use xt_ingest::native::saved_projects::list_saved_projects;

pub struct ProjectsService {
    /// The home folder whose saved files are read, or none (fixture mode).
    home: Option<PathBuf>,
}

impl ProjectsService {
    /// The service over the native `home` startup selected, or over none.
    /// Only the path is kept: nothing is read, checked or created.
    pub fn new(home: Option<&Path>) -> Self {
        Self {
            home: home.map(Path::to_path_buf),
        }
    }

    /// Read on the calling thread.
    pub fn list(&self) -> ProjectList {
        list_under(self.home.as_deref())
    }
}

fn list_under(home: Option<&Path>) -> ProjectList {
    match home {
        Some(home) => list_saved_projects(home).into(),
        None => ProjectList::NotConfigured,
    }
}

/// Read on the blocking pool. A worker that panics is `failed`.
pub async fn list_on_worker(service: &ProjectsService) -> ProjectList {
    let home = service.home.clone();
    tauri::async_runtime::spawn_blocking(move || list_under(home.as_deref()))
        .await
        .unwrap_or(ProjectList::Failed)
}
