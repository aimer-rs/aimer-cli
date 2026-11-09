//! Shared workspace host.
//!
//! `aimer mcp` normally becomes a thin stdio proxy to one long-lived host per
//! workspace. The host owns the analysis engine and serves every proxy over a
//! loopback TCP connection, so several agents on one workspace share a single
//! rust-analyzer instance. Tool calls are ordered by [`super::access`].

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

use super::AimerMcpServer;

const HOST_DIRECTORY: &str = ".aimer/mcp-hosts";
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const IDLE_CHECK: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const HOST_START_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_HANDSHAKE_BYTES: u64 = 4096;

/// How `aimer mcp` serves its client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServeMode {
    /// Proxy to the workspace host, starting it when needed.
    Shared,
    /// Serve one client in this process, without sharing the engine.
    Standalone,
    /// Run as the workspace host (started by a proxy).
    Host,
}

#[derive(Debug, Serialize, Deserialize)]
struct HostRecord {
    endpoint: String,
    token: String,
    pid: u32,
    project_root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct Handshake {
    token: String,
    client: String,
    full_output: bool,
}

fn host_directory() -> anyhow::Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(HOST_DIRECTORY))
        .ok_or_else(|| anyhow!("could not determine the user home directory"))
}

/// Stable per-workspace file stem derived from the canonical root.
fn host_key(project_root: &Path) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    hex::encode(&digest[..8])
}

fn record_path(project_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(host_directory()?.join(format!("{}.json", host_key(project_root))))
}

fn lock_path(project_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(host_directory()?.join(format!("{}.lock", host_key(project_root))))
}

fn prepare_directory() -> anyhow::Result<PathBuf> {
    let directory = host_directory()?;
    fs::create_dir_all(&directory).with_context(|| format!("creating {}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory)
}

fn write_record(path: &Path, record: &HostRecord) -> anyhow::Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(record)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

fn read_record(path: &Path) -> Option<HostRecord> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Hold an exclusive advisory lock for the life of the process so only one
/// host runs per workspace. Returns `None` when another host holds it.
#[cfg(unix)]
fn acquire_host_lock(path: &Path) -> anyhow::Result<Option<fs::File>> {
    use std::os::fd::AsRawFd;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    Ok(locked.then_some(file))
}

#[cfg(not(unix))]
fn acquire_host_lock(path: &Path) -> anyhow::Result<Option<fs::File>> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    Ok(Some(file))
}

/// Run the workspace host until its last client has been gone for a while.
pub(super) async fn serve_host(server: AimerMcpServer) -> anyhow::Result<()> {
    let project_root = server.project_root.clone();
    prepare_directory()?;
    let Some(_lock) = acquire_host_lock(&lock_path(&project_root)?)? else {
        return Ok(());
    };
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("binding the MCP host endpoint")?;
    let token = Uuid::new_v4().to_string();
    let path = record_path(&project_root)?;
    write_record(
        &path,
        &HostRecord {
            endpoint: listener.local_addr()?.to_string(),
            token: token.clone(),
            pid: std::process::id(),
            project_root: project_root.clone(),
        },
    )?;

    let mut last_active = Instant::now();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                last_active = Instant::now();
                let server = server.clone();
                let token = token.clone();
                tokio::spawn(async move {
                    if let Err(error) = serve_connection(stream, server, &token).await {
                        tracing::debug!(error = %error, "MCP host connection ended");
                    }
                });
            }
            () = tokio::time::sleep(IDLE_CHECK) => {
                if server.gate.clients() > 0 {
                    last_active = Instant::now();
                } else if last_active.elapsed() >= IDLE_TIMEOUT {
                    break;
                }
            }
        }
    }
    if read_record(&path).is_some_and(|record| record.token == token) {
        let _ = fs::remove_file(&path);
    }
    Ok(())
}

async fn serve_connection(
    stream: TcpStream,
    server: AimerMcpServer,
    token: &str,
) -> anyhow::Result<()> {
    use rmcp::ServiceExt;

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let read = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        (&mut reader).take(MAX_HANDSHAKE_BYTES).read_line(&mut line),
    )
    .await
    .context("MCP host handshake timed out")??;
    anyhow::ensure!(read > 0, "MCP host client closed before the handshake");
    let handshake: Handshake =
        serde_json::from_str(line.trim()).context("invalid MCP host handshake")?;
    if handshake.token != token {
        writer.write_all(b"denied\n").await.ok();
        anyhow::bail!("MCP host handshake used the wrong token");
    }
    writer.write_all(b"ok\n").await?;

    let client = handshake.client.chars().take(36).collect::<String>();
    let connection = server.gate.connect();
    let bridge = server.rust_analyzer.clone();
    let mut server = server;
    server.client_id = client.clone();
    server.full_output = handshake.full_output;
    // Each client is its own agent: it holds its own lease, which ends with it.
    server.remote = std::sync::Arc::default();
    let service = server.serve((reader, writer)).await?;
    let result = service.waiting().await;
    drop(connection);
    bridge.release_client(&client).await;
    result?;
    Ok(())
}

/// Proxy this process's stdio to the workspace host, starting the host when
/// none is running. Returns `Ok(false)` when no host could be reached, so the
/// caller can serve standalone.
pub(super) fn run_proxy(project_root: &Path, full_output: bool) -> anyhow::Result<bool> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("starting MCP proxy runtime")?;
    runtime.block_on(async {
        let Some(stream) = connect_or_start(project_root, full_output).await? else {
            return Ok(false);
        };
        pump(stream).await?;
        Ok(true)
    })
}

async fn connect_or_start(
    project_root: &Path,
    full_output: bool,
) -> anyhow::Result<Option<BufReader<TcpStream>>> {
    if let Some(stream) = try_connect(project_root, full_output).await {
        return Ok(Some(stream));
    }
    if spawn_host(project_root).is_err() {
        return Ok(None);
    }
    let deadline = Instant::now() + HOST_START_TIMEOUT;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(stream) = try_connect(project_root, full_output).await {
            return Ok(Some(stream));
        }
    }
    Ok(None)
}

async fn try_connect(project_root: &Path, full_output: bool) -> Option<BufReader<TcpStream>> {
    let record = read_record(&record_path(project_root).ok()?)?;
    let attempt = async {
        let mut stream = TcpStream::connect(&record.endpoint).await.ok()?;
        let handshake = Handshake {
            token: record.token,
            client: Uuid::new_v4().to_string(),
            full_output,
        };
        let mut line = serde_json::to_vec(&handshake).ok()?;
        line.push(b'\n');
        stream.write_all(&line).await.ok()?;
        let mut stream = BufReader::new(stream);
        let mut reply = String::new();
        stream.read_line(&mut reply).await.ok()?;
        (reply.trim() == "ok").then_some(stream)
    };
    tokio::time::timeout(HANDSHAKE_TIMEOUT, attempt)
        .await
        .ok()
        .flatten()
}

fn spawn_host(project_root: &Path) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command
        .arg("mcp")
        .arg("--host")
        .arg("--project")
        .arg(project_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("starting the MCP workspace host")?;
    Ok(())
}

async fn pump(stream: BufReader<TcpStream>) -> anyhow::Result<()> {
    let (mut host_reader, mut host_writer) = tokio::io::split(stream);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let upstream = async {
        let copied = tokio::io::copy(&mut stdin, &mut host_writer).await;
        host_writer.shutdown().await.ok();
        copied
    };
    let downstream = async {
        let copied = tokio::io::copy(&mut host_reader, &mut stdout).await;
        stdout.flush().await.ok();
        copied
    };
    tokio::select! {
        result = upstream => { result?; }
        result = downstream => { result?; }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_is_stable_and_distinguishes_workspaces() {
        let one = std::env::temp_dir();
        let two = one.join("aimer-host-key-other");
        assert_eq!(host_key(&one), host_key(&one));
        assert_ne!(host_key(&one), host_key(&two));
        assert_eq!(host_key(&one).len(), 16);
    }

    #[test]
    fn record_round_trips_through_a_private_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("host.json");
        let record = HostRecord {
            endpoint: "127.0.0.1:1".to_owned(),
            token: "t".to_owned(),
            pid: 7,
            project_root: directory.path().to_path_buf(),
        };
        write_record(&path, &record).unwrap();
        let read = read_record(&path).unwrap();
        assert_eq!((read.endpoint.as_str(), read.pid), ("127.0.0.1:1", 7));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
