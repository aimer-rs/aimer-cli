//! Session listing for the `sessions` command.

use std::path::PathBuf;

use anyhow::Context;

use crate::session::{SessionDescriptor, SessionRegistry};

/// List live session descriptors for the sessions command.
pub fn list_sessions(project: Option<PathBuf>, json_output: bool) -> anyhow::Result<()> {
    let project = project.map(resolve_project_path).transpose()?;
    let registry = SessionRegistry::default_location()?;
    let sessions = registry.live(project.as_deref())?;
    if json_output {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!("No live Aimer run sessions.");
        return Ok(());
    }
    for session in sessions {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            session.session_id,
            session.status_name(),
            session.target,
            session
                .app_process
                .as_ref()
                .and_then(|process| process.pid)
                .map_or_else(|| "-".to_string(), |pid| pid.to_string()),
            session.project_root.display(),
        );
    }
    Ok(())
}

pub(crate) fn resolve_project_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    path.canonicalize()
        .with_context(|| format!("project path '{}' does not exist", path.display()))
}

impl SessionDescriptor {
    fn status_name(&self) -> &'static str {
        match self.status {
            crate::session::SessionStatus::Starting => "starting",
            crate::session::SessionStatus::Locking => "locking",
            crate::session::SessionStatus::Fetching => "fetching",
            crate::session::SessionStatus::Compiling => "compiling",
            crate::session::SessionStatus::Building => "building",
            crate::session::SessionStatus::Launching => "launching",
            crate::session::SessionStatus::Running => "running",
            crate::session::SessionStatus::Idling => "idling",
            crate::session::SessionStatus::Restarting => "restarting",
            crate::session::SessionStatus::Stopping => "stopping",
            crate::session::SessionStatus::Stopped => "stopped",
            crate::session::SessionStatus::Error => "error",
        }
    }
}
