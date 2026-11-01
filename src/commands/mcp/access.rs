//! Access classes and the per-workspace gate that lets many MCP clients share
//! one analysis engine: any number of read tools run together, while a write
//! tool runs alone.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(60);
const BUILD_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(300);

/// What a tool needs from the shared workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// Pure queries and previews; compatible with every other read.
    Read,
    /// Cargo and flycheck runs: compatible with reads, exclusive among builds.
    Build,
    /// Changes workspace files or the analysis root; runs alone.
    Write,
    /// Controls the running Aimer app; exclusive among session controls.
    Session,
}

/// Declared access class for a tool name. Unknown tools are treated as writes
/// so a new tool is safe until it is classified.
pub(super) fn access_class(tool: &str) -> Access {
    match tool {
        "apply_edit" | "start_lsp" => Access::Write,
        "cargo_check" | "cargo_test" | "cargo_run" | "check" => Access::Build,
        "restart_project"
        | "stop_project"
        | "clear_logs"
        | "set_resource_monitor_interval"
        | "record_resource_usage" => Access::Session,
        "get_manifest"
        | "get_project_metadata"
        | "preview_manifest_edit"
        | "get_code_actions"
        | "rename_symbol"
        | "preview_code_action"
        | "preview_edit"
        | "format"
        | "search_symbols"
        | "search_text"
        | "inspect_file"
        | "get_document_symbols"
        | "completion"
        | "get_code_lenses"
        | "find_definition"
        | "find_declaration"
        | "find_references"
        | "find_implementations"
        | "type_definition"
        | "hover"
        | "document_highlights"
        | "signature_help"
        | "prepare_rename"
        | "inlay_hints"
        | "get_diagnostics"
        | "get_call_hierarchy"
        | "expand_macro"
        | "status"
        | "get_logs"
        | "get_resource_usage"
        | "list_resource_recordings"
        | "get_resource_recording"
        | "get_crate_docs"
        | "search_crate_docs" => Access::Read,
        _ => Access::Write,
    }
}

#[derive(Default)]
struct Holders {
    writer: StdMutex<Option<String>>,
    builder: StdMutex<Option<String>>,
    session: StdMutex<Option<String>>,
}

fn set(slot: &StdMutex<Option<String>>, value: Option<String>) {
    *slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

fn get(slot: &StdMutex<Option<String>>) -> Option<String> {
    slot.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Shared by every client connection of one workspace host.
///
/// The underlying `RwLock` is fair: a waiting writer blocks new readers, so a
/// steady stream of reads cannot starve a write.
#[derive(Clone, Default)]
pub(super) struct WorkspaceGate {
    lock: Arc<RwLock<()>>,
    build: Arc<Mutex<()>>,
    session: Arc<Mutex<()>>,
    holders: Arc<Holders>,
    connected: Arc<AtomicUsize>,
}

/// Counts a connected client until dropped.
pub(super) struct ClientGuard(Arc<AtomicUsize>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Held for the duration of one tool call; releases the gate on drop.
pub(super) struct Permit {
    _read: Option<OwnedRwLockReadGuard<()>>,
    _write: Option<OwnedRwLockWriteGuard<()>>,
    _exclusive: Option<OwnedMutexGuard<()>>,
    release: Option<(Arc<Holders>, Access)>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some((holders, access)) = self.release.take() {
            match access {
                Access::Write => set(&holders.writer, None),
                Access::Build => set(&holders.builder, None),
                Access::Session => set(&holders.session, None),
                Access::Read => {}
            }
        }
    }
}

impl WorkspaceGate {
    /// Wait for the access `tool` needs. Returns a `busy` message naming the
    /// current holder instead of blocking indefinitely.
    pub(super) async fn acquire(
        &self,
        access: Access,
        tool: &str,
        client: &str,
    ) -> Result<Permit, String> {
        let limit = if access == Access::Build {
            BUILD_ACQUIRE_TIMEOUT
        } else {
            ACQUIRE_TIMEOUT
        };
        match tokio::time::timeout(limit, self.acquire_unbounded(access, tool, client)).await {
            Ok(permit) => Ok(permit),
            Err(_) => Err(self.busy_message(access, tool)),
        }
    }

    async fn acquire_unbounded(&self, access: Access, tool: &str, client: &str) -> Permit {
        let who = Some(format!("{tool} for client {client}"));
        let mut permit = Permit {
            _read: None,
            _write: None,
            _exclusive: None,
            release: None,
        };
        match access {
            Access::Read => {
                permit._read = Some(self.lock.clone().read_owned().await);
            }
            Access::Build => {
                permit._exclusive = Some(self.build.clone().lock_owned().await);
                permit._read = Some(self.lock.clone().read_owned().await);
                set(&self.holders.builder, who);
                permit.release = Some((self.holders.clone(), access));
            }
            Access::Write => {
                permit._write = Some(self.lock.clone().write_owned().await);
                set(&self.holders.writer, who);
                permit.release = Some((self.holders.clone(), access));
            }
            Access::Session => {
                permit._exclusive = Some(self.session.clone().lock_owned().await);
                set(&self.holders.session, who);
                permit.release = Some((self.holders.clone(), access));
            }
        }
        permit
    }

    fn busy_message(&self, access: Access, tool: &str) -> String {
        let holder = match access {
            Access::Build => get(&self.holders.builder).or_else(|| get(&self.holders.writer)),
            Access::Session => get(&self.holders.session),
            Access::Read | Access::Write => get(&self.holders.writer)
                .or_else(|| get(&self.holders.builder))
                .map(|holder| holder.to_owned()),
        };
        match holder {
            Some(holder) => format!(
                "workspace is busy: '{tool}' waited too long behind {holder}; retry shortly"
            ),
            None => format!(
                "workspace is busy: '{tool}' waited too long for other clients; retry shortly"
            ),
        }
    }

    /// Mark a client connected until the returned guard drops.
    pub(super) fn connect(&self) -> ClientGuard {
        self.connected.fetch_add(1, Ordering::AcqRel);
        ClientGuard(self.connected.clone())
    }

    /// Number of currently connected clients.
    pub(super) fn clients(&self) -> usize {
        self.connected.load(Ordering::Acquire)
    }

    /// Connected clients and current lock holders, or `None` when the
    /// workspace is not shared and nothing is running.
    pub(super) fn describe(&self) -> Option<serde_json::Value> {
        let clients = self.clients();
        let (writer, builder, session) = self.holders();
        if clients <= 1 && writer.is_none() && builder.is_none() && session.is_none() {
            return None;
        }
        Some(serde_json::json!({
            "clients": clients,
            "writer": writer,
            "build": builder,
            "session": session,
        }))
    }

    /// Description of the active writer, builder and session controller.
    pub(super) fn holders(&self) -> (Option<String>, Option<String>, Option<String>) {
        (
            get(&self.holders.writer),
            get(&self.holders.builder),
            get(&self.holders.session),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_match_the_tool_effects() {
        assert_eq!(access_class("search_text"), Access::Read);
        assert_eq!(access_class("preview_edit"), Access::Read);
        assert_eq!(access_class("apply_edit"), Access::Write);
        assert_eq!(access_class("start_lsp"), Access::Write);
        assert_eq!(access_class("cargo_test"), Access::Build);
        assert_eq!(access_class("restart_project"), Access::Session);
        assert_eq!(access_class("brand_new_tool"), Access::Write);
    }

    #[tokio::test]
    async fn reads_share_and_a_write_is_alone() {
        let gate = WorkspaceGate::default();
        let first = gate.acquire(Access::Read, "hover", "a").await.unwrap();
        let second = gate.acquire(Access::Read, "hover", "b").await.unwrap();
        let blocked = tokio::time::timeout(
            Duration::from_millis(50),
            gate.acquire(Access::Write, "apply_edit", "c"),
        )
        .await;
        assert!(blocked.is_err(), "write must wait for readers");
        drop((first, second));
        let write = gate
            .acquire(Access::Write, "apply_edit", "c")
            .await
            .unwrap();
        assert_eq!(gate.holders().0.as_deref(), Some("apply_edit for client c"));
        let read_waits = tokio::time::timeout(
            Duration::from_millis(50),
            gate.acquire(Access::Read, "hover", "a"),
        )
        .await;
        assert!(read_waits.is_err(), "reads wait while a write runs");
        drop(write);
        assert!(gate.holders().0.is_none());
    }

    #[test]
    fn client_guard_tracks_connections() {
        let gate = WorkspaceGate::default();
        let first = gate.connect();
        let second = gate.connect();
        assert_eq!(gate.clients(), 2);
        drop((first, second));
        assert_eq!(gate.clients(), 0);
    }

    #[tokio::test]
    async fn builds_run_beside_reads_but_not_each_other() {
        let gate = WorkspaceGate::default();
        let build = gate
            .acquire(Access::Build, "cargo_test", "a")
            .await
            .unwrap();
        let _read = gate.acquire(Access::Read, "hover", "b").await.unwrap();
        let second = tokio::time::timeout(
            Duration::from_millis(50),
            gate.acquire(Access::Build, "check", "b"),
        )
        .await;
        assert!(second.is_err());
        drop(build);
    }
}
