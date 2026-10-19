//! Running-app resource sampling and retained per-run recordings.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use uuid::Uuid;

use crate::config::ResourceMonitorConfig;
use crate::session::{ProcessIdentity, ProcessScope, SessionStatus};

pub const MIN_SAMPLE_INTERVAL_MS: u64 = 500;
pub const MAX_SAMPLE_INTERVAL_MS: u64 = 3_000;
const DEFAULT_PAGE_SIZE: usize = 100;
const MAX_PAGE_SIZE: usize = 2_000;

/// One timestamped reading for the process Aimer launched.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceSample {
    /// UTC time in milliseconds since the Unix epoch.
    pub timestamp_unix_ms: u64,
    /// PID on the host or device running the app.
    pub pid: Option<u32>,
    /// Process CPU usage. It can exceed 100% when multiple cores are used.
    pub cpu_percent: Option<f32>,
    /// Per-process GPU usage when the platform provides it.
    pub gpu_percent: Option<f32>,
    /// Resident set size in bytes.
    pub memory_bytes: Option<u64>,
    /// Number of process threads when the platform provides it.
    pub thread_count: Option<usize>,
    /// Explanations for metrics unavailable in this sample.
    pub unavailable: Vec<String>,
}

/// Latest monitor state exposed to the TUI and the live MCP endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceMonitorStatus {
    pub interval_ms: u64,
    pub recording: bool,
    pub sample: Option<ResourceSample>,
}

impl Default for ResourceMonitorStatus {
    fn default() -> Self {
        Self {
            interval_ms: ResourceMonitorConfig::default().sample_interval_ms,
            recording: false,
            sample: None,
        }
    }
}

/// Metadata for one Aimer run's resource recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceRecordingMetadata {
    pub recording_id: String,
    pub session_id: Uuid,
    pub run_id: u64,
    pub target: String,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub sample_count: u64,
}

/// A cursor-paginated page of recording metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceRecordingsPage {
    pub recordings: Vec<ResourceRecordingMetadata>,
    pub next_cursor: Option<u64>,
}

/// A cursor-paginated page of timestamped samples from one recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourceSamplesPage {
    pub recording: ResourceRecordingMetadata,
    pub samples: Vec<ResourceSample>,
    pub next_cursor: Option<u64>,
}

/// A per-project store under the user's `~/.aimer` directory.
#[derive(Clone, Debug)]
pub struct ResourceRecordStore {
    root: PathBuf,
    project_root: PathBuf,
}

impl ResourceRecordStore {
    /// Open the default user's recording store for a project.
    pub fn for_project(project_root: &Path) -> anyhow::Result<Self> {
        let home = dirs::home_dir().ok_or_else(|| anyhow!("home directory is unavailable"))?;
        Ok(Self::new(
            home.join(".aimer/resource-recordings"),
            project_root.to_path_buf(),
        ))
    }

    /// Create a store at an explicit location, primarily for isolated callers.
    pub fn new(root: PathBuf, project_root: PathBuf) -> Self {
        Self { root, project_root }
    }

    fn project_directory(&self) -> PathBuf {
        let normalized =
            fs::canonicalize(&self.project_root).unwrap_or_else(|_| self.project_root.clone());
        let digest = Sha256::digest(normalized.to_string_lossy().as_bytes());
        self.root.join(hex::encode(&digest[..16]))
    }

    /// Begin recording one session run. The ID remains stable across reads.
    pub fn begin(
        &self,
        session_id: Uuid,
        run_id: u64,
        target: impl Into<String>,
        retention_days: u32,
    ) -> anyhow::Result<ResourceRecorder> {
        self.prune_expired(retention_days)?;
        let directory = self.project_directory();
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "creating resource recording directory {}",
                directory.display()
            )
        })?;
        let recording_id = format!("{session_id}_{run_id}");
        let samples_path = directory.join(format!("{recording_id}.jsonl"));
        let metadata_path = directory.join(format!("{recording_id}.meta.json"));
        let metadata = ResourceRecordingMetadata {
            recording_id,
            session_id,
            run_id,
            target: target.into(),
            started_at_unix_ms: now_unix_ms(),
            finished_at_unix_ms: None,
            sample_count: 0,
        };
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&samples_path)
            .with_context(|| format!("opening resource recording {}", samples_path.display()))?;
        write_metadata(&metadata_path, &metadata)?;
        Ok(ResourceRecorder {
            metadata,
            metadata_path,
            samples_path,
        })
    }

    /// List recent recordings for this project, pruning expired completed runs.
    pub fn list(
        &self,
        retention_days: u32,
        cursor: Option<u64>,
        limit: Option<usize>,
    ) -> anyhow::Result<ResourceRecordingsPage> {
        self.prune_expired(retention_days)?;
        let directory = self.project_directory();
        let mut recordings = Vec::new();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ResourceRecordingsPage {
                    recordings,
                    next_cursor: None,
                });
            }
            Err(error) => return Err(error).context("reading resource recordings"),
        };
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("json")
            {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !name.ends_with(".meta.json") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Ok(metadata) = serde_json::from_str(&contents) {
                recordings.push(metadata);
            }
        }
        recordings.sort_by(|left: &ResourceRecordingMetadata, right| {
            right
                .started_at_unix_ms
                .cmp(&left.started_at_unix_ms)
                .then_with(|| right.recording_id.cmp(&left.recording_id))
        });
        let offset = cursor.unwrap_or(0) as usize;
        let page_size = limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, DEFAULT_PAGE_SIZE);
        let next_cursor = (offset.saturating_add(page_size) < recordings.len())
            .then_some(offset.saturating_add(page_size) as u64);
        let page = recordings
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect();
        Ok(ResourceRecordingsPage {
            recordings: page,
            next_cursor,
        })
    }

    /// Read samples from a single per-project recording.
    pub fn read(
        &self,
        recording_id: &str,
        cursor: Option<u64>,
        limit: Option<usize>,
        retention_days: u32,
    ) -> anyhow::Result<ResourceSamplesPage> {
        self.prune_expired(retention_days)?;
        validate_recording_id(recording_id)?;
        let directory = self.project_directory();
        let metadata_path = directory.join(format!("{recording_id}.meta.json"));
        let samples_path = directory.join(format!("{recording_id}.jsonl"));
        let metadata: ResourceRecordingMetadata = serde_json::from_slice(
            &fs::read(&metadata_path)
                .with_context(|| format!("resource recording '{recording_id}' was not found"))?,
        )
        .context("decoding resource recording metadata")?;
        let offset = cursor.unwrap_or(0) as usize;
        let page_size = limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
        let reader = BufReader::new(
            File::open(&samples_path)
                .with_context(|| format!("opening resource recording '{recording_id}'"))?,
        );
        let mut samples = Vec::with_capacity(page_size);
        let mut next_cursor = None;
        for (index, line) in reader.lines().enumerate() {
            if index < offset {
                continue;
            }
            let line = line.context("reading resource sample")?;
            if samples.len() == page_size {
                next_cursor = Some(index as u64);
                break;
            }
            samples.push(serde_json::from_str(&line).context("decoding resource sample")?);
        }
        Ok(ResourceSamplesPage {
            recording: metadata,
            samples,
            next_cursor,
        })
    }

    /// Remove completed recordings older than the configured retention period.
    pub fn prune_expired(&self, retention_days: u32) -> anyhow::Result<()> {
        let directory = self.project_directory();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error).context("reading resource recording directory"),
        };
        let cutoff = now_unix_ms()
            .saturating_sub(u64::from(retention_days).saturating_mul(24 * 60 * 60 * 1_000));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".meta.json"))
            {
                continue;
            }
            let Ok(contents) = fs::read(&path) else {
                continue;
            };
            let Ok(metadata) = serde_json::from_slice::<ResourceRecordingMetadata>(&contents)
            else {
                continue;
            };
            // Keep a long-running active recording even if it crosses the age
            // boundary; it becomes eligible after it is finalized.
            if metadata.finished_at_unix_ms.is_some()
                && metadata.finished_at_unix_ms.unwrap_or_default() < cutoff
            {
                let _ = fs::remove_file(&path);
                let samples_path = path.with_file_name(format!("{}.jsonl", metadata.recording_id));
                let _ = fs::remove_file(samples_path);
            }
        }
        Ok(())
    }
}

/// Append-only writer for one active run.
pub struct ResourceRecorder {
    metadata: ResourceRecordingMetadata,
    metadata_path: PathBuf,
    samples_path: PathBuf,
}

impl ResourceRecorder {
    pub fn recording_id(&self) -> &str {
        &self.metadata.recording_id
    }

    pub fn append(&mut self, sample: &ResourceSample) -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.samples_path)
            .with_context(|| {
                format!("opening resource recording {}", self.samples_path.display())
            })?;
        serde_json::to_writer(&mut file, sample).context("encoding resource sample")?;
        file.write_all(b"\n")?;
        file.flush()?;
        self.metadata.sample_count = self.metadata.sample_count.saturating_add(1);
        write_metadata(&self.metadata_path, &self.metadata)
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.metadata.finished_at_unix_ms.is_none() {
            self.metadata.finished_at_unix_ms = Some(now_unix_ms());
            write_metadata(&self.metadata_path, &self.metadata)?;
        }
        Ok(())
    }
}

impl Drop for ResourceRecorder {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn write_metadata(path: &Path, metadata: &ResourceRecordingMetadata) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("resource recording metadata path has no file name"))?;
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    serde_json::to_writer(File::create(&temporary)?, metadata)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn validate_recording_id(recording_id: &str) -> anyhow::Result<()> {
    let Some((session_id, run_id)) = recording_id.rsplit_once('_') else {
        return Err(anyhow!("invalid resource recording ID"));
    };
    Uuid::parse_str(session_id).context("invalid resource recording session ID")?;
    run_id
        .parse::<u64>()
        .context("invalid resource recording run ID")?;
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Input copied from a live session before a collector call.
#[derive(Clone, Debug)]
pub(crate) struct MonitorInputs {
    pub session_id: Uuid,
    pub run_id: u64,
    pub project_root: PathBuf,
    pub target: String,
    pub device: Option<String>,
    pub status: SessionStatus,
    pub app_process: Option<ProcessIdentity>,
    pub interval_ms: u64,
    pub retention_days: u32,
}

/// State passed back to a live session after collection and persistence.
#[derive(Clone, Debug)]
pub(crate) struct MonitorUpdate {
    pub run_id: u64,
    pub sample: ResourceSample,
    pub recording: bool,
}

/// Sample a local process with sysinfo and remote Android processes through adb.
/// iOS device process counters are reported unavailable because the host CLI
/// cannot inspect a sandboxed device process through sysinfo.
pub struct ResourceSampler {
    system: System,
    sampled_pids: HashSet<Pid>,
    android_cpu: HashMap<u32, (u64, Instant)>,
    #[cfg(target_os = "macos")]
    macos_gpu_time: HashMap<u32, (u64, Instant)>,
}

impl ResourceSampler {
    pub fn new() -> Self {
        Self {
            system: System::new(),
            sampled_pids: HashSet::new(),
            android_cpu: HashMap::new(),
            #[cfg(target_os = "macos")]
            macos_gpu_time: HashMap::new(),
        }
    }

    pub fn sample(
        &mut self,
        target: &str,
        device: Option<&str>,
        process: &ProcessIdentity,
        interval_ms: u64,
    ) -> ResourceSample {
        match process.scope {
            ProcessScope::Host => self.sample_host(process),
            ProcessScope::Device if target.starts_with("android") => {
                self.sample_android(device, process.pid, interval_ms)
            }
            ProcessScope::Device if target.starts_with("ios") => unavailable_device_sample(
                process.pid,
                "iOS device sandboxing prevents the host from reading app process counters",
            ),
            ProcessScope::Device => unavailable_device_sample(
                process.pid,
                "device process metrics are unavailable through the host sysinfo collector",
            ),
        }
    }

    fn sample_host(&mut self, process: &ProcessIdentity) -> ResourceSample {
        let Some(pid) = process.pid else {
            return unavailable_device_sample(None, "the app process PID is not available");
        };
        let sys_pid = Pid::from(pid as usize);
        let refresh = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_tasks();
        self.system
            .refresh_processes_specifics(ProcessesToUpdate::Some(&[sys_pid]), true, refresh);
        let Some(process_info) = self.system.process(sys_pid) else {
            return unavailable_device_sample(
                Some(pid),
                "the process is no longer visible to sysinfo",
            );
        };
        let cpu_percent = if self.sampled_pids.insert(sys_pid) {
            None
        } else {
            Some(process_info.cpu_usage())
        };
        let thread_count = process_info
            .tasks()
            .map(|tasks| tasks.len())
            .or_else(|| platform_thread_count(pid));
        #[cfg(target_os = "macos")]
        let (gpu_percent, gpu_unavailable) =
            sample_macos_gpu_percent(pid, &mut self.macos_gpu_time);
        #[cfg(not(target_os = "macos"))]
        let (gpu_percent, gpu_unavailable) = (
            None,
            Some("sysinfo does not expose per-process GPU utilization".to_string()),
        );
        let mut unavailable = gpu_unavailable
            .map(|reason| vec![format!("gpu: {reason}")])
            .unwrap_or_default();
        if cpu_percent.is_none() {
            unavailable.push("cpu: waiting for the second process refresh".to_string());
        }
        if thread_count.is_none() {
            unavailable
                .push("threads: this platform does not expose a process thread count".to_string());
        }
        ResourceSample {
            timestamp_unix_ms: now_unix_ms(),
            pid: Some(pid),
            cpu_percent,
            gpu_percent,
            memory_bytes: Some(process_info.memory()),
            thread_count,
            unavailable,
        }
    }

    fn sample_android(
        &mut self,
        device: Option<&str>,
        pid: Option<u32>,
        _interval_ms: u64,
    ) -> ResourceSample {
        let Some(pid) = pid else {
            return unavailable_device_sample(None, "Android app PID is not available");
        };
        let script = format!(
            "cat /proc/{pid}/stat; printf '\\n--AIMER-STATUS--\\n'; cat /proc/{pid}/status"
        );
        let mut command = Command::new("adb");
        if let Some(device) =
            device.filter(|id| !id.is_empty() && *id != "local" && *id != "default")
        {
            command.args(["-s", device]);
        }
        let output = command.args(["shell", "sh", "-c", &script]).output();
        let parsed = output
            .map_err(|error| error.to_string())
            .and_then(|output| {
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
                }
                parse_android_process_info(&String::from_utf8_lossy(&output.stdout))
            });
        let mut unavailable =
            vec!["gpu: sysinfo does not expose per-process GPU utilization".to_string()];
        match parsed {
            Ok((cpu_ticks, memory_bytes, thread_count)) => {
                let now = Instant::now();
                let cpu_percent = self.android_cpu.insert(pid, (cpu_ticks, now)).and_then(
                    |(previous_ticks, previous_time)| {
                        let elapsed = now.duration_since(previous_time).as_secs_f32();
                        (elapsed > 0.0).then(|| {
                            cpu_ticks.saturating_sub(previous_ticks) as f32 * 100.0
                                / (100.0 * elapsed)
                        })
                    },
                );
                if cpu_percent.is_none() {
                    unavailable
                        .push("cpu: waiting for the second Android process sample".to_string());
                }
                ResourceSample {
                    timestamp_unix_ms: now_unix_ms(),
                    pid: Some(pid),
                    cpu_percent,
                    gpu_percent: None,
                    memory_bytes: Some(memory_bytes),
                    thread_count: Some(thread_count),
                    unavailable,
                }
            }
            Err(error) => {
                unavailable.push(format!("Android process metrics: {error}"));
                ResourceSample {
                    timestamp_unix_ms: now_unix_ms(),
                    pid: Some(pid),
                    cpu_percent: None,
                    gpu_percent: None,
                    memory_bytes: None,
                    thread_count: None,
                    unavailable,
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn sample_macos_gpu_percent(
    pid: u32,
    previous: &mut HashMap<u32, (u64, Instant)>,
) -> (Option<f32>, Option<String>) {
    let output = match Command::new("ioreg")
        .args(["-r", "-c", "AGXDeviceUserClient", "-l", "-w0"])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if error.is_empty() {
                format!("ioreg exited with status {}", output.status)
            } else {
                format!("ioreg: {error}")
            };
            return (None, Some(detail));
        }
        Err(error) => return (None, Some(format!("could not run ioreg: {error}"))),
    };
    // AGX user clients publish cumulative GPU time alongside their owning PID.
    let output = String::from_utf8_lossy(&output.stdout);
    let Some(gpu_time_ns) = parse_macos_ioreg_gpu_time(&output, pid) else {
        previous.remove(&pid);
        return (
            None,
            Some("no per-process Apple GPU usage record was exposed for this PID".to_string()),
        );
    };

    let now = Instant::now();
    let current = previous.insert(pid, (gpu_time_ns, now));
    let Some((previous_gpu_time_ns, previous_at)) = current else {
        return (
            None,
            Some("waiting for the second macOS GPU sample".to_string()),
        );
    };
    if gpu_time_ns < previous_gpu_time_ns {
        return (
            None,
            Some("macOS GPU counter reset; waiting for the next sample".to_string()),
        );
    }
    let elapsed_ns = now.duration_since(previous_at).as_nanos();
    if elapsed_ns == 0 {
        return (
            None,
            Some("macOS GPU sample interval was too short".to_string()),
        );
    }
    let gpu_percent =
        ((gpu_time_ns - previous_gpu_time_ns) as f64 / elapsed_ns as f64 * 100.0) as f32;
    (Some(gpu_percent), None)
}

#[cfg(target_os = "macos")]
fn parse_macos_ioreg_gpu_time(output: &str, pid: u32) -> Option<u64> {
    let mut app_usage_time_ns = None;
    let mut total_time_ns = 0u64;
    let mut matching_client = false;

    for line in output.lines() {
        if let Some(app_usage) = line.split_once("\"AppUsage\" =") {
            app_usage_time_ns = Some(parse_accumulated_gpu_time(app_usage.1));
        }
        if let Some(creator) = line.split_once("\"IOUserClientCreator\" = \"pid ") {
            let client_pid = creator
                .1
                .split(|character| matches!(character, ',' | ' ' | '"'))
                .next()
                .and_then(|value| value.parse::<u32>().ok());
            if client_pid == Some(pid) {
                matching_client = true;
                total_time_ns =
                    total_time_ns.saturating_add(app_usage_time_ns.take().unwrap_or_default());
            } else {
                app_usage_time_ns = None;
            }
        }
    }

    matching_client.then_some(total_time_ns)
}

#[cfg(target_os = "macos")]
fn parse_accumulated_gpu_time(app_usage: &str) -> u64 {
    let mut remaining = app_usage;
    let mut total = 0u64;
    while let Some((_, value)) = remaining.split_once("accumulatedGPUTime") {
        let Some((_, value)) = value.split_once('=') else {
            break;
        };
        let digits = value.trim_start().trim_start_matches('"');
        let digit_count = digits
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digit_count == 0 {
            remaining = value;
            continue;
        }
        if let Ok(time_ns) = digits[..digit_count].parse::<u64>() {
            total = total.saturating_add(time_ns);
        }
        remaining = &digits[digit_count..];
    }
    total
}

fn unavailable_device_sample(pid: Option<u32>, reason: &str) -> ResourceSample {
    ResourceSample {
        timestamp_unix_ms: now_unix_ms(),
        pid,
        cpu_percent: None,
        gpu_percent: None,
        memory_bytes: None,
        thread_count: None,
        unavailable: vec![
            format!("process metrics: {reason}"),
            "gpu: sysinfo does not expose per-process GPU utilization".to_string(),
        ],
    }
}

fn parse_android_process_info(output: &str) -> Result<(u64, u64, usize), String> {
    let (stat, status) = output
        .split_once("--AIMER-STATUS--")
        .ok_or_else(|| "adb output did not include process status".to_string())?;
    let stat = stat.trim();
    let after_name = stat
        .rfind(')')
        .map(|index| &stat[index + 1..])
        .ok_or_else(|| "malformed Android process stat".to_string())?;
    let fields: Vec<&str> = after_name.split_whitespace().collect();
    let user_ticks = fields
        .get(11)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| "Android process stat has no user CPU time".to_string())?;
    let system_ticks = fields
        .get(12)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| "Android process stat has no system CPU time".to_string())?;
    let mut memory_kb = None;
    let mut threads = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            memory_kb = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok());
        } else if let Some(value) = line.strip_prefix("Threads:") {
            threads = value.trim().parse::<usize>().ok();
        }
    }
    let memory_bytes = memory_kb
        .ok_or_else(|| "Android process status has no resident memory value".to_string())?
        .saturating_mul(1024);
    let threads =
        threads.ok_or_else(|| "Android process status has no thread count".to_string())?;
    Ok((
        user_ticks.saturating_add(system_ticks),
        memory_bytes,
        threads,
    ))
}

#[cfg(target_os = "macos")]
fn platform_thread_count(pid: u32) -> Option<usize> {
    let output = Command::new("ps")
        .args(["-M", "-p", &pid.to_string(), "-o", "pid="])
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
    })
}

#[cfg(target_os = "windows")]
fn platform_thread_count(pid: u32) -> Option<usize> {
    let script = format!("(Get-Process -Id {pid}).Threads.Count");
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_thread_count(_pid: u32) -> Option<usize> {
    None
}

pub(crate) fn run_monitor(
    handle: crate::session::SessionHandle,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    let mut sampler = ResourceSampler::new();
    let mut store = None;
    let mut recorder: Option<ResourceRecorder> = None;
    let mut active_run_id: Option<u64> = None;
    let mut last_sample = Instant::now();

    while !shutdown.load(Ordering::Acquire) {
        let inputs = handle.monitor_inputs();
        let active_process = matches!(inputs.status, SessionStatus::Running | SessionStatus::Error)
            .then_some(inputs.app_process.as_ref())
            .flatten()
            .filter(|process| process.role == crate::session::ProcessRole::App);
        if let Some(process) = active_process {
            if active_run_id != Some(inputs.run_id) {
                if let Some(mut old) = recorder.take() {
                    let _ = old.finish();
                }
                active_run_id = Some(inputs.run_id);
                if store.is_none() {
                    store = ResourceRecordStore::for_project(&inputs.project_root)
                        .map_err(|error| {
                            tracing::debug!(error = %error, "could not open resource recording store");
                            error
                        })
                        .ok();
                }
                recorder = store.as_ref().and_then(|store| {
                    store
                        .begin(
                            inputs.session_id,
                            inputs.run_id,
                            inputs.target.clone(),
                            inputs.retention_days,
                        )
                        .map_err(|error| tracing::debug!(error = %error, "could not start resource recording"))
                        .ok()
                });
                last_sample = Instant::now() - Duration::from_millis(inputs.interval_ms);
            }
            if last_sample.elapsed() >= Duration::from_millis(inputs.interval_ms) {
                let sample = sampler.sample(
                    &inputs.target,
                    inputs.device.as_deref(),
                    process,
                    inputs.interval_ms,
                );
                let recording = recorder.as_mut().is_some_and(|recorder| {
                    recorder
                        .append(&sample)
                        .map_err(|error| tracing::debug!(error = %error, "could not append resource sample"))
                        .is_ok()
                });
                handle.publish_resource_sample(MonitorUpdate {
                    run_id: inputs.run_id,
                    sample,
                    recording,
                });
                last_sample = Instant::now();
            }
        } else {
            active_run_id = None;
            if let Some(mut old) = recorder.take() {
                let _ = old.finish();
            }
            handle.clear_resource_sample();
            last_sample = Instant::now();
        }

        let elapsed = last_sample.elapsed();
        let delay = if active_process.is_some() {
            Duration::from_millis(inputs.interval_ms).saturating_sub(elapsed)
        } else {
            Duration::from_millis(200)
        };
        handle.wait_for_monitor_wake(delay);
    }
    if let Some(mut recorder) = recorder {
        let _ = recorder.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_android_process_stat_and_status() {
        let output = "123 (jaime app) S 1 2 3 4 5 6 7 8 9 10 120 30 16 17 18 19 20 21 22 23 24\n--AIMER-STATUS--\nName:\tjaime\nVmRSS:\t 4096 kB\nThreads:\t7\n";
        assert_eq!(
            parse_android_process_info(output).unwrap(),
            (150, 4_194_304, 7)
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn sysinfo_reads_host_memory_and_process_cpu_after_two_refreshes() {
        let pid = std::process::id();
        let identity = ProcessIdentity::host_app(pid, None, None);
        let mut sampler = ResourceSampler::new();

        let first = sampler.sample("macos", None, &identity, 1_000);
        assert_eq!(first.pid, Some(pid));
        assert!(first.memory_bytes.is_some());
        assert_eq!(first.cpu_percent, None);

        let second = sampler.sample("macos", None, &identity, 1_000);
        assert!(second.cpu_percent.is_some());
    }

    #[test]
    fn recording_store_writes_and_pages_samples() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let store = ResourceRecordStore::new(temp.path().join("store"), project);
        let session = Uuid::new_v4();
        let mut recorder = store.begin(session, 1, "macos", 30).unwrap();
        let id = recorder.recording_id().to_string();
        for sample in 0..3 {
            recorder
                .append(&ResourceSample {
                    timestamp_unix_ms: sample,
                    pid: Some(42),
                    cpu_percent: Some(12.5),
                    gpu_percent: None,
                    memory_bytes: Some(1024),
                    thread_count: Some(4),
                    unavailable: vec!["gpu: unavailable".into()],
                })
                .unwrap();
        }
        recorder.finish().unwrap();
        let page = store.read(&id, Some(1), Some(1), 30).unwrap();
        assert_eq!(page.samples.len(), 1);
        assert_eq!(page.samples[0].timestamp_unix_ms, 1);
        assert_eq!(page.next_cursor, Some(2));
        let listed = store.list(30, None, None).unwrap();
        assert_eq!(listed.recordings.len(), 1);
        assert_eq!(listed.recordings[0].sample_count, 3);
    }
}
