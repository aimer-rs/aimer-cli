use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::Context;
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::time::{interval, timeout};

use super::super::mcp_ra::McpProgressReporter;
use super::super::run::cargo_message::{CargoMessage, DiagnosticLevel};
use super::common::{CARGO_TIMEOUT, MAX_TOOL_OUTPUT_BYTES, bounded_text, find_lockfile};
use super::metadata::normalize_path;

const DEFAULT_CARGO_RUN_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CARGO_RUN_TIMEOUT_SECONDS: u64 = 300;
const MAX_CLIPPY_DIAGNOSTICS: usize = 200;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct CargoRunArguments {
    /// Optional Cargo package name. Omit to run across the selected workspace.
    #[serde(default)]
    pub package: Option<String>,
    /// Optional target triple, for example `x86_64-unknown-linux-gnu`.
    #[serde(default)]
    pub target: Option<String>,
    /// Features to enable.
    #[serde(default)]
    pub features: Vec<String>,
    /// Enable every available feature.
    #[serde(default)]
    pub all_features: bool,
    /// Disable default features.
    #[serde(default)]
    pub no_default_features: bool,
    /// Require Cargo.lock to remain current. Defaults to Cargo's normal behavior.
    #[serde(default)]
    pub locked: bool,
    /// Optional Cargo test name filter. Used by `cargo_test` only.
    #[serde(default)]
    pub test_filter: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct CargoRunToolArguments {
    /// Optional Cargo package name. Omit to use Cargo's default package selection.
    #[serde(default)]
    pub package: Option<String>,
    /// Optional binary target name.
    #[serde(default)]
    pub bin: Option<String>,
    /// Optional example target name. Mutually exclusive with `bin`.
    #[serde(default)]
    pub example: Option<String>,
    /// Optional target triple, for example `x86_64-unknown-linux-gnu`.
    #[serde(default)]
    pub target: Option<String>,
    /// Features to enable.
    #[serde(default)]
    pub features: Vec<String>,
    /// Enable every available feature.
    #[serde(default)]
    pub all_features: bool,
    /// Disable default features.
    #[serde(default)]
    pub no_default_features: bool,
    /// Build in release mode.
    #[serde(default)]
    pub release: bool,
    /// Require Cargo.lock to remain current.
    #[serde(default)]
    pub locked: bool,
    /// Arguments passed to the program after `--`.
    #[serde(default)]
    pub program_args: Vec<String>,
    /// Maximum run duration, including compilation (1–300 seconds; defaults to 60).
    #[serde(default)]
    #[schemars(range(min = 1, max = 300))]
    pub timeout_seconds: Option<u64>,
}

pub async fn run_cargo(
    root: &Path,
    allow_network: bool,
    is_test: bool,
    arguments: CargoRunArguments,
    progress: Option<&McpProgressReporter>,
) -> anyhow::Result<Value> {
    validate_cargo_arguments(is_test, &arguments)?;
    let command_name = if is_test { "test" } else { "check" };
    let lockfile = find_lockfile(root);
    let lock_before = match &lockfile {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };

    let mut command = Command::new("cargo");
    command
        .arg(command_name)
        .arg("--message-format=json")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true);
    if let Some(package) = arguments.package.as_deref() {
        command.args(["--package", package]);
    } else {
        command.arg("--workspace");
    }
    if let Some(target) = arguments.target.as_deref() {
        command.args(["--target", target]);
    }
    if !arguments.features.is_empty() {
        command.args(["--features", &arguments.features.join(",")]);
    }
    if arguments.all_features {
        command.arg("--all-features");
    }
    if arguments.no_default_features {
        command.arg("--no-default-features");
    }
    if arguments.locked {
        command.arg("--locked");
    }
    if let Some(filter) = arguments.test_filter.as_deref() {
        command.arg(filter);
    }

    let command_label = format!("cargo {command_name}");
    if let Some(progress) = progress {
        progress
            .report(format!(
                "Running {command_label} for the selected Rust workspace."
            ))
            .await;
    }
    let started = Instant::now();
    let output_future = timeout(CARGO_TIMEOUT, command.output());
    tokio::pin!(output_future);
    let mut progress_interval = interval(Duration::from_secs(10));
    progress_interval.tick().await;
    let output_result = loop {
        tokio::select! {
            result = &mut output_future => break result,
            _ = progress_interval.tick() => {
                if let Some(progress) = progress {
                    progress.report(format!("{command_label} is still running.")).await;
                }
            }
        }
    };
    let (status, exit_code, stdout, stderr, warnings_filtered, cargo_progress_lines_filtered) =
        match output_result {
            Ok(Ok(output)) => {
                let (filtered_stderr, cargo_progress_lines_filtered) =
                    filter_cargo_progress_lines(&output.stderr);
                let (stdout, stderr, warnings_filtered) =
                    normalize_cargo_output(&output.stdout, filtered_stderr.as_bytes());
                (
                    if output.status.success() {
                        "passed"
                    } else {
                        "failed"
                    },
                    output.status.code(),
                    bounded_text(&stdout),
                    bounded_text(&stderr),
                    warnings_filtered,
                    cargo_progress_lines_filtered,
                )
            }
            Ok(Err(error)) => return Err(error).context(format!("starting {command_label}")),
            Err(_) => (
                "timed_out",
                None,
                (String::new(), false),
                (String::new(), false),
                0,
                0,
            ),
        };
    let lockfile_changed = if let Some(path) = lockfile {
        let lock_after = tokio::fs::read(path).await.ok();
        lock_before != lock_after
    } else {
        false
    };
    Ok(json!({
        "operation": command_name,
        "status": status,
        "exit_code": exit_code,
        "duration_ms": started.elapsed().as_millis(),
        "stdout": stdout.0,
        "stdout_truncated": stdout.1,
        "stderr": stderr.0,
        "stderr_truncated": stderr.1,
        "warnings_filtered": warnings_filtered,
        "cargo_progress_lines_filtered": cargo_progress_lines_filtered,
        "lockfile_changed": lockfile_changed,
        "timeout_seconds": CARGO_TIMEOUT.as_secs()
    }))
}

pub async fn run_cargo_binary(
    root: &Path,
    allow_network: bool,
    arguments: CargoRunToolArguments,
    progress: Option<&McpProgressReporter>,
) -> anyhow::Result<Value> {
    validate_cargo_run_arguments(&arguments)?;
    let timeout_duration = Duration::from_secs(
        arguments
            .timeout_seconds
            .unwrap_or(DEFAULT_CARGO_RUN_TIMEOUT.as_secs()),
    );
    let lockfile = find_lockfile(root);
    let lock_before = match &lockfile {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };

    let mut command = Command::new("cargo");
    command
        .args(["run", "--message-format=json"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true);
    if let Some(package) = arguments.package.as_deref() {
        command.args(["--package", package]);
    }
    if let Some(bin) = arguments.bin.as_deref() {
        command.args(["--bin", bin]);
    }
    if let Some(example) = arguments.example.as_deref() {
        command.args(["--example", example]);
    }
    if let Some(target) = arguments.target.as_deref() {
        command.args(["--target", target]);
    }
    if !arguments.features.is_empty() {
        command.args(["--features", &arguments.features.join(",")]);
    }
    if arguments.all_features {
        command.arg("--all-features");
    }
    if arguments.no_default_features {
        command.arg("--no-default-features");
    }
    if arguments.release {
        command.arg("--release");
    }
    if arguments.locked {
        command.arg("--locked");
    }
    if !arguments.program_args.is_empty() {
        command.arg("--").args(&arguments.program_args);
    }

    if let Some(progress) = progress {
        progress
            .report("Running cargo run for the selected Rust project. Cargo build chatter and compiler warnings will be filtered from the result.")
            .await;
    }
    let started = Instant::now();
    let output_future = timeout(timeout_duration, command.output());
    tokio::pin!(output_future);
    let mut progress_interval = interval(Duration::from_secs(10));
    progress_interval.tick().await;
    let output_result = loop {
        tokio::select! {
            result = &mut output_future => break result,
            _ = progress_interval.tick() => {
                if let Some(progress) = progress {
                    progress.report("cargo run is still running.").await;
                }
            }
        }
    };
    let (status, exit_code, stdout, stderr, warnings_filtered, cargo_progress_lines_filtered) =
        match output_result {
            Ok(Ok(output)) => {
                let (filtered_stderr, cargo_progress_lines_filtered) =
                    filter_cargo_progress_lines(&output.stderr);
                let (stdout, stderr, warnings_filtered) =
                    normalize_cargo_output_filtering_build_warnings(
                        &output.stdout,
                        filtered_stderr.as_bytes(),
                    );
                (
                    if output.status.success() {
                        "passed"
                    } else {
                        "failed"
                    },
                    output.status.code(),
                    bounded_text(&stdout),
                    bounded_text(&stderr),
                    warnings_filtered,
                    cargo_progress_lines_filtered,
                )
            }
            Ok(Err(error)) => return Err(error).context("starting cargo run"),
            Err(_) => (
                "timed_out",
                None,
                (String::new(), false),
                (String::new(), false),
                0,
                0,
            ),
        };
    let lockfile_changed = if let Some(path) = lockfile {
        tokio::fs::read(path).await.ok() != lock_before
    } else {
        false
    };
    Ok(json!({
        "operation": "run",
        "status": status,
        "exit_code": exit_code,
        "duration_ms": started.elapsed().as_millis(),
        "stdout": stdout.0,
        "stdout_truncated": stdout.1,
        "stderr": stderr.0,
        "stderr_truncated": stderr.1,
        "warnings_filtered": warnings_filtered,
        "cargo_progress_lines_filtered": cargo_progress_lines_filtered,
        "lockfile_changed": lockfile_changed,
        "timeout_seconds": timeout_duration.as_secs()
    }))
}

pub(super) fn validate_cargo_run_arguments(
    arguments: &CargoRunToolArguments,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !(arguments.bin.is_some() && arguments.example.is_some()),
        "bin and example cannot be selected together"
    );
    for (name, value) in [
        ("package", arguments.package.as_deref()),
        ("bin", arguments.bin.as_deref()),
        ("example", arguments.example.as_deref()),
        ("target", arguments.target.as_deref()),
    ] {
        anyhow::ensure!(
            value.is_none_or(|value| !value.trim().is_empty()),
            "{name} must not be empty"
        );
        anyhow::ensure!(
            value.is_none_or(|value| !value.starts_with('-')),
            "{name} must be a value, not an option"
        );
    }
    anyhow::ensure!(
        arguments
            .features
            .iter()
            .all(|feature| !feature.trim().is_empty() && !feature.starts_with('-')),
        "features must be non-empty feature names, not options"
    );
    anyhow::ensure!(
        !(arguments.all_features && !arguments.features.is_empty()),
        "features and all_features cannot be used together"
    );
    anyhow::ensure!(
        arguments.program_args.len() <= 128,
        "program_args cannot contain more than 128 items"
    );
    let args_size = arguments
        .program_args
        .iter()
        .map(String::len)
        .sum::<usize>();
    anyhow::ensure!(
        args_size <= MAX_TOOL_OUTPUT_BYTES,
        "program_args exceed the {MAX_TOOL_OUTPUT_BYTES}-byte limit"
    );
    anyhow::ensure!(
        arguments
            .timeout_seconds
            .is_none_or(|seconds| (1..=MAX_CARGO_RUN_TIMEOUT_SECONDS).contains(&seconds)),
        "timeout_seconds must be between 1 and {MAX_CARGO_RUN_TIMEOUT_SECONDS}"
    );
    Ok(())
}

/// Run Clippy directly through Cargo so the result includes Cargo's real exit
/// status. Every lint warning is promoted to an error by `-D warnings`.
pub async fn run_clippy(
    root: &Path,
    allow_network: bool,
    file_filter: Option<&Path>,
    progress: Option<&McpProgressReporter>,
) -> anyhow::Result<Value> {
    let root = root
        .canonicalize()
        .context("resolving Rust workspace root")?;
    let file_filter = file_filter
        .map(|file| {
            let path = if file.is_absolute() {
                file.to_path_buf()
            } else {
                root.join(file)
            };
            let path = path
                .canonicalize()
                .with_context(|| format!("resolving Rust source '{}'", path.display()))?;
            anyhow::ensure!(
                path.starts_with(&root),
                "file filter must stay inside the selected Rust project"
            );
            Ok::<_, anyhow::Error>(path)
        })
        .transpose()?;
    let lockfile = find_lockfile(&root);
    let lock_before = match &lockfile {
        Some(path) => tokio::fs::read(path).await.ok(),
        None => None,
    };
    let mut command = Command::new("cargo");
    command
        .args([
            "clippy",
            "--workspace",
            "--message-format=json",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "CARGO_NET_OFFLINE",
            if allow_network { "false" } else { "true" },
        )
        .kill_on_drop(true);
    if let Some(progress) = progress {
        progress
            .report("Running cargo clippy --workspace -- -D warnings.")
            .await;
    }
    let started = Instant::now();
    let output_future = timeout(CARGO_TIMEOUT, command.output());
    tokio::pin!(output_future);
    let mut progress_interval = interval(Duration::from_secs(10));
    progress_interval.tick().await;
    let output_result = loop {
        tokio::select! {
            result = &mut output_future => break result,
            _ = progress_interval.tick() => if let Some(progress) = progress {
                progress.report("cargo clippy is still running.").await;
            }
        }
    };
    let (status, exit_code, output) = match output_result {
        Ok(Ok(output)) => (
            if output.status.success() {
                "passed"
            } else {
                "failed"
            },
            output.status.code(),
            Some(output),
        ),
        Ok(Err(error)) => return Err(error).context("starting cargo clippy"),
        Err(_) => ("timed_out", None, None),
    };
    let mut diagnostics = Vec::new();
    let mut raw_stdout = String::new();
    let mut stderr = String::new();
    let mut cargo_progress_lines_filtered = 0;
    if let Some(output) = output {
        let (filtered_stderr, filtered_count) = filter_cargo_progress_lines(&output.stderr);
        stderr = filtered_stderr;
        cargo_progress_lines_filtered = filtered_count;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                raw_stdout.push_str(line);
                raw_stdout.push('\n');
                continue;
            };
            if message.get("reason").and_then(Value::as_str) != Some("compiler-message") {
                continue;
            }
            let Some(diagnostic) = message.get("message") else {
                continue;
            };
            let spans = diagnostic
                .get("spans")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let primary = spans
                .iter()
                .find(|span| span.get("is_primary").and_then(Value::as_bool) == Some(true));
            let path = primary
                .and_then(|span| span.get("file_name"))
                .and_then(Value::as_str);
            let normalized_path = path.map(|path| normalize_path(&root, path));
            if let (Some(filter), Some(path)) = (&file_filter, path) {
                let candidate = Path::new(path);
                let candidate = if candidate.is_absolute() {
                    candidate.to_path_buf()
                } else {
                    root.join(candidate)
                };
                if candidate.canonicalize().ok().as_deref() != Some(filter.as_path()) {
                    continue;
                }
            }
            let severity = diagnostic
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("warning");
            let code = diagnostic.pointer("/code/code").and_then(Value::as_str);
            diagnostics.push(json!({
                "file": normalized_path,
                "line": primary.and_then(|span| span.get("line_start")).cloned().unwrap_or(Value::Null),
                "column": primary.and_then(|span| span.get("column_start")).cloned().unwrap_or(Value::Null),
                "severity": severity,
                "source": if code.is_some_and(|code| code.starts_with("clippy::")) { "clippy" } else { "rustc" },
                "code": code,
                "message": diagnostic.get("message").and_then(Value::as_str).unwrap_or_default(),
                "rendered": diagnostic.get("rendered").and_then(Value::as_str)
                    .map(|rendered| rendered.chars().take(3000).collect::<String>())
            }));
        }
    }
    diagnostics.sort_by(|left, right| {
        left["file"]
            .as_str()
            .cmp(&right["file"].as_str())
            .then_with(|| left["line"].as_u64().cmp(&right["line"].as_u64()))
            .then_with(|| left["code"].as_str().cmp(&right["code"].as_str()))
    });
    let error_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .count();
    let truncated = diagnostics.len() > MAX_CLIPPY_DIAGNOSTICS;
    diagnostics.truncate(MAX_CLIPPY_DIAGNOSTICS);
    let lockfile_changed = if let Some(path) = lockfile {
        tokio::fs::read(path).await.ok() != lock_before
    } else {
        false
    };
    let (stdout, stdout_truncated) = bounded_text(raw_stdout.as_bytes());
    let (stderr, stderr_truncated) = bounded_text(stderr.as_bytes());
    Ok(json!({
        "operation": "cargo_clippy",
        "command": "cargo clippy --workspace -- -D warnings",
        "warnings_denied": true,
        "status": status,
        "exit_code": exit_code,
        "duration_ms": started.elapsed().as_millis(),
        "diagnostic_summary": { "error_count": error_count, "warning_count": warning_count },
        "diagnostics": diagnostics,
        "diagnostics_truncated": truncated,
        "stdout": stdout,
        "stdout_truncated": stdout_truncated,
        "stderr": stderr,
        "stderr_truncated": stderr_truncated,
        "cargo_progress_lines_filtered": cargo_progress_lines_filtered,
        "lockfile_changed": lockfile_changed,
        "timeout_seconds": CARGO_TIMEOUT.as_secs()
    }))
}

/// Remove rustc warning diagnostics from Cargo's JSON output while retaining
/// readable diagnostics and test-process output for the MCP result.
pub(super) fn normalize_cargo_output(stdout: &[u8], stderr: &[u8]) -> (Vec<u8>, Vec<u8>, usize) {
    normalize_cargo_output_with_build_warning_policy(stdout, stderr, true)
}

pub(super) fn normalize_cargo_output_filtering_build_warnings(
    stdout: &[u8],
    stderr: &[u8],
) -> (Vec<u8>, Vec<u8>, usize) {
    normalize_cargo_output_with_build_warning_policy(stdout, stderr, false)
}

pub(super) fn normalize_cargo_output_with_build_warning_policy(
    stdout: &[u8],
    stderr: &[u8],
    keep_build_script_warnings: bool,
) -> (Vec<u8>, Vec<u8>, usize) {
    let stdout = String::from_utf8_lossy(stdout);
    let mut visible_stdout = String::new();
    let mut visible_stderr = String::from_utf8_lossy(stderr).into_owned();
    let mut warnings_filtered = 0;

    for raw_line in stdout.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            visible_stdout.push_str(raw_line);
            continue;
        };
        let Some(reason) = message.get("reason").and_then(Value::as_str) else {
            visible_stdout.push_str(raw_line);
            continue;
        };

        match reason {
            "compiler-message" => match CargoMessage::parse(line) {
                Some(CargoMessage::Diagnostic(diagnostic))
                    if diagnostic.level == DiagnosticLevel::Warning =>
                {
                    warnings_filtered += 1;
                }
                Some(CargoMessage::Diagnostic(diagnostic)) => {
                    append_diagnostic(&mut visible_stderr, &diagnostic.render());
                }
                _ => visible_stdout.push_str(raw_line),
            },
            // These are Cargo's machine protocol, not useful command output.
            "compiler-artifact" | "build-finished" => {}
            // Build-script output is Cargo metadata; callers can choose whether
            // its warnings should be retained in the user-facing result.
            "build-script-executed" => {
                if let Some(output) = message.get("out").and_then(Value::as_str) {
                    for line in output.lines() {
                        if let Some(warning) = line
                            .strip_prefix("cargo:warning=")
                            .or_else(|| line.strip_prefix("cargo::warning="))
                        {
                            if keep_build_script_warnings {
                                append_diagnostic(
                                    &mut visible_stderr,
                                    &format!("warning: {warning}"),
                                );
                            } else {
                                warnings_filtered += 1;
                            }
                        }
                    }
                }
            }
            _ => visible_stdout.push_str(raw_line),
        }
    }

    (
        visible_stdout.into_bytes(),
        visible_stderr.into_bytes(),
        warnings_filtered,
    )
}

pub(super) fn append_diagnostic(output: &mut String, diagnostic: &str) {
    if diagnostic.is_empty() {
        return;
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(diagnostic);
    if !diagnostic.ends_with('\n') {
        output.push('\n');
    }
}

/// Remove low-signal Cargo build/run progress lines from stderr while keeping
/// diagnostics and program output intact.
pub(super) fn filter_cargo_progress_lines(stderr: &[u8]) -> (String, usize) {
    let stderr = String::from_utf8_lossy(stderr);
    let mut visible = String::new();
    let mut filtered_count = 0;
    for raw_line in stderr.split_inclusive('\n') {
        let line = raw_line.trim_start().trim_end_matches(['\r', '\n']);
        if ["Compiling ", "Checking ", "Finished ", "Running `"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            filtered_count += 1;
        } else {
            visible.push_str(raw_line);
        }
    }
    (visible, filtered_count)
}

pub(super) fn validate_cargo_arguments(
    is_test: bool,
    arguments: &CargoRunArguments,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        arguments
            .package
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "package must not be empty"
    );
    anyhow::ensure!(
        arguments
            .package
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "package must be a package name, not an option"
    );
    anyhow::ensure!(
        arguments
            .target
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "target must not be empty"
    );
    anyhow::ensure!(
        arguments
            .target
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "target must be a target triple, not an option"
    );
    anyhow::ensure!(
        arguments
            .features
            .iter()
            .all(|feature| !feature.trim().is_empty() && !feature.starts_with('-')),
        "features must be non-empty feature names, not options"
    );
    anyhow::ensure!(
        arguments
            .test_filter
            .as_deref()
            .is_none_or(|value| !value.trim().is_empty()),
        "test_filter must not be empty"
    );
    anyhow::ensure!(
        arguments
            .test_filter
            .as_deref()
            .is_none_or(|value| !value.starts_with('-')),
        "test_filter must be a test name, not an option"
    );
    anyhow::ensure!(
        is_test || arguments.test_filter.is_none(),
        "test_filter is only valid for cargo_test"
    );
    anyhow::ensure!(
        !(arguments.all_features && !arguments.features.is_empty()),
        "features and all_features cannot be used together"
    );
    Ok(())
}
