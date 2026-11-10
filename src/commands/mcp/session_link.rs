//! The Aimer session one MCP connection controls.
//!
//! A link starts from the session picked when the server started and can change
//! while the connection lives. In [`LinkMode::Automatic`] it follows the project's
//! live session: exactly one live session is attached, and the session going away
//! is noticed. An explicit attach pins one session; a detach leaves the link empty.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::session::{ControlClient, SessionDescriptor, SessionRegistry};

/// The least time between two looks at the registry while a session is attached.
const RESOLVE_INTERVAL: Duration = Duration::from_secs(1);

/// How the link decides which session it controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkMode {
    /// Follows the project's live session when there is exactly one.
    Automatic,
    /// Stays on the session an explicit attach chose.
    Pinned,
    /// Controls no session until an attach asks for one.
    Detached,
}

impl LinkMode {
    fn name(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Pinned => "pinned",
            Self::Detached => "detached",
        }
    }
}

/// What automatic resolution does with the sessions that are alive.
#[derive(Debug, PartialEq, Eq)]
enum Choice {
    Keep,
    Switch(Uuid),
    Clear,
}

/// Exactly one live session is the one to control. With none, the attached
/// session is gone. With several there is no way to choose, so the attached one
/// stays while it is among them.
fn choose(live: &[Uuid], current: Option<Uuid>) -> Choice {
    match live {
        [only] if current == Some(*only) => Choice::Keep,
        [only] => Choice::Switch(*only),
        _ => match current {
            Some(current) if !live.contains(&current) => Choice::Clear,
            _ => Choice::Keep,
        },
    }
}

struct LinkState {
    client: Option<ControlClient>,
    mode: LinkMode,
    /// Counts the changes of session, so a claim made on the old one can be told apart.
    epoch: u64,
    checked: Option<Instant>,
}

pub(super) struct SessionLink {
    project_root: PathBuf,
    registry: Option<SessionRegistry>,
    state: Mutex<LinkState>,
}

impl SessionLink {
    pub(super) fn new(
        project_root: PathBuf,
        registry: Option<SessionRegistry>,
        client: Option<ControlClient>,
        mode: LinkMode,
    ) -> Self {
        Self {
            project_root,
            registry,
            state: Mutex::new(LinkState {
                client,
                mode,
                epoch: 0,
                checked: None,
            }),
        }
    }

    /// A link for another connection that starts where this one is now.
    pub(super) fn fork(&self) -> Self {
        let state = self.lock();
        Self {
            project_root: self.project_root.clone(),
            registry: self.registry.clone(),
            state: Mutex::new(LinkState {
                client: state.client.clone(),
                mode: state.mode,
                epoch: state.epoch,
                checked: None,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The session to control, after looking again in automatic mode, and the
    /// epoch of that session. This reads the registry, so call it off the async
    /// threads.
    pub(super) fn snapshot(&self) -> (Option<ControlClient>, u64) {
        let mut state = self.lock();
        if state.mode == LinkMode::Automatic && self.due(&state) {
            state.checked = Some(Instant::now());
            self.resolve(&mut state);
        }
        (state.client.clone(), state.epoch)
    }

    pub(super) fn current(&self) -> Option<ControlClient> {
        self.snapshot().0
    }

    fn due(&self, state: &LinkState) -> bool {
        state.client.is_none()
            || state
                .checked
                .is_none_or(|at| at.elapsed() >= RESOLVE_INTERVAL)
    }

    fn resolve(&self, state: &mut LinkState) {
        let Some(registry) = &self.registry else {
            return;
        };
        let Ok(live) = registry.live(Some(&self.project_root)) else {
            return;
        };
        let ids: Vec<Uuid> = live.iter().map(|session| session.session_id).collect();
        let current = state
            .client
            .as_ref()
            .map(|client| client.descriptor().session_id);
        match choose(&ids, current) {
            Choice::Keep => {}
            Choice::Clear => {
                state.client = None;
                state.epoch += 1;
            }
            Choice::Switch(id) => {
                if let Ok(client) = registry.attach(id) {
                    state.client = Some(client);
                    state.epoch += 1;
                }
            }
        }
    }

    fn registry(&self) -> anyhow::Result<&SessionRegistry> {
        self.registry
            .as_ref()
            .ok_or_else(|| anyhow!("the Aimer session registry is not available"))
    }

    /// The live sessions of this project, or of every project.
    pub(super) fn list(&self, all_projects: bool) -> anyhow::Result<Vec<SessionDescriptor>> {
        let project: Option<&Path> = (!all_projects).then_some(self.project_root.as_path());
        self.registry()?.live(project)
    }

    /// Controls exactly this session from now on, whatever else is alive.
    pub(super) fn pin(&self, id: Uuid) -> anyhow::Result<ControlClient> {
        let client = self.registry()?.attach(id)?;
        let mut state = self.lock();
        state.client = Some(client.clone());
        state.mode = LinkMode::Pinned;
        state.epoch += 1;
        Ok(client)
    }

    /// Goes back to following the project's live session, and looks right away.
    pub(super) fn follow_project(&self) -> Option<ControlClient> {
        let mut state = self.lock();
        state.mode = LinkMode::Automatic;
        state.checked = Some(Instant::now());
        self.resolve(&mut state);
        state.client.clone()
    }

    /// Controls no session until the next attach.
    pub(super) fn detach(&self) {
        let mut state = self.lock();
        state.client = None;
        state.mode = LinkMode::Detached;
        state.epoch += 1;
    }

    /// The mode and the session being controlled, for a tool reply.
    pub(super) fn describe(&self) -> Value {
        let (client, _) = self.snapshot();
        let mode = self.lock().mode;
        json!({
            "mode": mode.name(),
            "session": client.as_ref().map(|client| session_summary(client.descriptor())),
        })
    }
}

/// The identity of a session, without the parts that change while it runs.
pub(super) fn session_summary(descriptor: &SessionDescriptor) -> Value {
    json!({
        "session_id": descriptor.session_id,
        "project_root": descriptor.project_root,
        "target": descriptor.target,
        "device": descriptor.device,
    })
}

/// Parses a session ID given to a tool.
pub(super) fn parse_id(value: &str) -> anyhow::Result<Uuid> {
    Uuid::parse_str(value.trim()).with_context(|| format!("invalid session ID '{value}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn exactly_one_live_session_is_attached() {
        assert_eq!(choose(&[id(1)], None), Choice::Switch(id(1)));
        assert_eq!(choose(&[id(2)], Some(id(1))), Choice::Switch(id(2)));
        assert_eq!(choose(&[id(1)], Some(id(1))), Choice::Keep);
    }

    #[test]
    fn no_live_session_clears_the_attached_one() {
        assert_eq!(choose(&[], Some(id(1))), Choice::Clear);
        assert_eq!(choose(&[], None), Choice::Keep);
    }

    #[test]
    fn several_live_sessions_never_pick_one_for_the_agent() {
        assert_eq!(choose(&[id(1), id(2)], None), Choice::Keep);
        assert_eq!(choose(&[id(1), id(2)], Some(id(2))), Choice::Keep);
        assert_eq!(choose(&[id(1), id(2)], Some(id(3))), Choice::Clear);
    }

    fn empty_link(mode: LinkMode) -> (tempfile::TempDir, SessionLink) {
        let directory = tempfile::tempdir().unwrap();
        let registry = SessionRegistry::new(directory.path().join("sessions"));
        let link = SessionLink::new(directory.path().to_path_buf(), Some(registry), None, mode);
        (directory, link)
    }

    #[test]
    fn an_automatic_link_without_sessions_controls_nothing() {
        let (_directory, link) = empty_link(LinkMode::Automatic);
        assert!(link.current().is_none());
        assert_eq!(link.describe()["mode"], "automatic");
        assert!(link.describe()["session"].is_null());
    }

    #[test]
    fn detaching_leaves_the_link_empty_and_changes_the_epoch() {
        let (_directory, link) = empty_link(LinkMode::Automatic);
        let (_, before) = link.snapshot();
        link.detach();
        let (client, after) = link.snapshot();
        assert!(client.is_none());
        assert_ne!(before, after);
        assert_eq!(link.describe()["mode"], "detached");
    }

    #[test]
    fn following_the_project_again_switches_back_to_automatic() {
        let (_directory, link) = empty_link(LinkMode::Detached);
        assert!(link.follow_project().is_none());
        assert_eq!(link.describe()["mode"], "automatic");
    }

    #[test]
    fn pinning_a_session_that_does_not_exist_fails_and_keeps_the_mode() {
        let (_directory, link) = empty_link(LinkMode::Automatic);
        assert!(link.pin(id(9)).is_err());
        assert_eq!(link.describe()["mode"], "automatic");
    }

    #[test]
    fn a_forked_link_starts_in_the_same_mode_and_then_moves_alone() {
        let (_directory, link) = empty_link(LinkMode::Pinned);
        let fork = link.fork();
        assert_eq!(fork.describe()["mode"], "pinned");
        fork.detach();
        assert_eq!(link.describe()["mode"], "pinned");
    }

    #[test]
    fn session_ids_are_parsed_exactly() {
        assert_eq!(parse_id(&format!(" {} ", id(5))).unwrap(), id(5));
        assert!(parse_id("not-an-id").is_err());
    }
}
