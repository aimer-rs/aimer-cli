//! MCP adapter for Aimer's embedded rust-analyzer IDE engine.
//!
//! The MCP process owns the selected in-process analysis database and exposes
//! compact JSON results while keeping existing tool names and arguments stable.

mod document_search;
mod file_inspection;
#[cfg(feature = "legacy-lsp")]
mod legacy_lsp;
mod lsp_values;
mod macro_references;
mod operations;
mod previews;
mod result_normalization;
mod source_edits;

use lsp_values::*;
use macro_references::*;
use source_edits::*;

pub(crate) use document_search::path_glob_matches;
pub use document_search::search_text;
pub use file_inspection::inspect_file;
pub use result_normalization::{compact_document_symbols, normalize_bounded, normalize_result};

#[cfg(feature = "legacy-lsp")]
use std::collections::HashSet;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use regex::RegexBuilder;
use rmcp::model::{ProgressNotificationParam, ProgressToken};
use rmcp::service::RequestContext;
use rmcp::{Peer, RoleServer, schemars};
use serde_json::{Map, Value, json};
use syn::visit::Visit;
use tokio::io::AsyncWriteExt;
#[cfg(feature = "legacy-lsp")]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
#[cfg(feature = "legacy-lsp")]
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;
use uuid::Uuid;

use super::mcp_ra_engine::EmbeddedRustAnalyzer;

const MAX_RESULTS: usize = 200;
const MAX_DOCUMENT_SYMBOLS: usize = 500;
const MAX_DOCUMENT_SYMBOL_DEPTH: usize = 8;
const MAX_DOCUMENT_SYMBOL_DETAIL_CHARS: usize = 300;
const MAX_INSPECT_RANGES: usize = 10;
const MAX_INSPECT_SEARCHES: usize = 10;
const MAX_INSPECT_RANGE_LINES: usize = 300;
const MAX_INSPECT_MATCHES: usize = 100;
const MAX_INSPECT_CONTEXT_LINES: usize = 5;
const MAX_INSPECT_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_INSPECT_LINE_CHARS: usize = 500;
#[cfg(feature = "legacy-lsp")]
const MAX_LSP_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
#[cfg(feature = "legacy-lsp")]
const MAX_PENDING_LSP_MESSAGES: usize = 128;
#[cfg(feature = "legacy-lsp")]
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
#[cfg(feature = "legacy-lsp")]
const INDEXING_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(feature = "legacy-lsp")]
const INDEXING_POLL_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(feature = "legacy-lsp")]
const INDEXING_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
#[cfg(feature = "legacy-lsp")]
const QUIESCENT_STABILITY_WINDOW: Duration = Duration::from_secs(1);
#[cfg(feature = "legacy-lsp")]
const INITIAL_QUIESCENT_STABILITY_WINDOW: Duration = Duration::from_secs(3);
#[cfg(feature = "legacy-lsp")]
const CHECK_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(feature = "legacy-lsp")]
const CHECK_START_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(feature = "legacy-lsp")]
const CHECK_SETTLE_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(feature = "legacy-lsp")]
const FLYCHECK_PROGRESS_TOKEN_PREFIX: &str = "rust-analyzer/flycheck/";

/// Sends MCP progress while a tool waits for rust-analyzer's workspace index.
pub struct McpProgressReporter {
    peer: Peer<RoleServer>,
    token: Option<ProgressToken>,
    next_progress: AtomicU64,
}

impl McpProgressReporter {
    pub fn from_context(context: &RequestContext<RoleServer>) -> Self {
        Self {
            peer: context.peer.clone(),
            token: context.meta.get_progress_token(),
            next_progress: AtomicU64::new(1),
        }
    }

    pub async fn report(&self, message: impl Into<String>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let progress = self.next_progress.fetch_add(1, Ordering::Relaxed) as f64;
        let notification = ProgressNotificationParam::new(token, progress).with_message(message);
        if let Err(error) = self.peer.notify_progress(notification).await {
            tracing::debug!(error = %error, "could not report rust-analyzer indexing progress to MCP client");
        }
    }
}

#[derive(Clone)]
pub struct RustAnalyzerBridge {
    project_root: Arc<std::sync::RwLock<PathBuf>>,
    config: aimer_hot_reload::config::RustAnalyzerConfig,
    session: Arc<Mutex<Option<EmbeddedRustAnalyzer>>>,
    actions: Arc<Mutex<HashMap<String, Value>>>,
    previews: Arc<Mutex<HashMap<String, EditPreview>>>,
}

#[derive(Clone, Debug)]
pub struct InspectFileRange {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug)]
pub struct InspectFilePattern {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub context_before: usize,
    pub context_after: usize,
}

/// A project file operation requested by the preview_edit MCP tool.
#[derive(Clone, Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SourceFileChange {
    /// Replace exact text anchors in an existing file. Each anchor must occur once.
    Edit {
        /// Project-relative file path.
        file: String,
        /// Exact old and new text pairs, all matched against the original file.
        replacements: Vec<TextReplacement>,
    },
    /// Create a new file in an existing project directory.
    Create {
        /// Project-relative file path that does not already exist.
        file: String,
        /// Complete UTF-8 file contents.
        content: String,
    },
    /// Delete an existing project file.
    Delete {
        /// Project-relative file path.
        file: String,
    },
}

/// One exact text replacement inside a source edit.
#[derive(Clone, Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TextReplacement {
    /// Non-empty exact text that must occur once in the original file.
    pub old_text: String,
    /// Replacement text. An empty value removes the matched text.
    pub new_text: String,
}

#[cfg(test)]
mod tests;
