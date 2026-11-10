use super::session_link::{parse_id, session_summary};
use super::*;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ListSessionsArguments {
    /// List the live sessions of every project instead of only this workspace's.
    #[serde(default)]
    all_projects: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct AttachSessionArguments {
    /// The session to control, as listed by list_sessions. Omit it to go back to
    /// following this workspace's live session automatically.
    #[serde(default)]
    session_id: Option<String>,
}

/// Runs blocking session work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| anyhow!("session lookup failed: {error}"))?
}

#[tool_router(router = session_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        name = "list_sessions",
        description = "List the live Aimer sessions of this workspace (or of every project with all_projects), marking the one this connection controls, and say whether the connection follows the workspace's session automatically, is pinned to one, or is detached."
    )]
    async fn list_sessions(
        &self,
        Parameters(arguments): Parameters<ListSessionsArguments>,
    ) -> CallToolResult {
        let link = self.link.clone();
        let all_projects = arguments.all_projects.unwrap_or(false);
        let result = blocking(move || {
            let sessions = link.list(all_projects)?;
            let mut reply = link.describe();
            let attached = reply["session"]["session_id"].as_str().map(str::to_owned);
            let sessions: Vec<Value> = sessions
                .iter()
                .map(|session| {
                    let mut entry = session_summary(session);
                    entry["status"] = json!(session.status);
                    entry["run_id"] = json!(session.run_id);
                    entry["attached"] =
                        json!(attached.as_deref() == Some(session.session_id.to_string().as_str()));
                    entry
                })
                .collect();
            reply["sessions"] = Value::Array(sessions);
            Ok(reply)
        })
        .await;
        encode("list_sessions", result)
    }

    #[tool(
        name = "attach_session",
        description = "Control the given live Aimer session from now on, so the session tools (status, restart_project, get_logs, the app-driving tools) act on it. Omit session_id to go back to following the workspace's live session automatically: with exactly one live session it is attached, and a session that ends is let go. Only this connection changes. A session of another project is allowed; the Rust tools keep using this workspace."
    )]
    async fn attach_session(
        &self,
        Parameters(arguments): Parameters<AttachSessionArguments>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let link = self.link.clone();
        let result = blocking(move || {
            match arguments.session_id.as_deref() {
                Some(id) => {
                    link.pin(parse_id(id)?)?;
                }
                None => {
                    link.follow_project();
                }
            }
            Ok(link.describe())
        })
        .await;
        self.session_changed(&context).await;
        encode("attach_session", result)
    }

    #[tool(
        name = "detach_session",
        description = "Stop controlling any Aimer session on this connection, and stop following the workspace's live session automatically, until attach_session is called. Only this connection changes."
    )]
    async fn detach_session(&self, context: RequestContext<RoleServer>) -> CallToolResult {
        self.link.detach();
        self.session_changed(&context).await;
        encode("detach_session", Ok(self.link.describe()))
    }
}

impl AimerMcpServer {
    /// Lets go of the claim made on the old session, and tells the client its
    /// tool list may have changed with the session.
    async fn session_changed(&self, context: &RequestContext<RoleServer>) {
        self.remote.release();
        // The client is gone when this fails, which needs no more than that.
        let _ = context.peer.notify_tool_list_changed().await;
    }
}
