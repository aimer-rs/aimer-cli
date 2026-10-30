use super::*;

pub struct SessionRuntime {
    handle: SessionHandle,
    shutdown: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    monitor_join: Option<JoinHandle<()>>,
}

/// Stable name for the run-scoped control lifetime exposed to embedders.
pub type RunSession = SessionRuntime;

impl SessionRuntime {
    /// Start a private loopback endpoint and persist its discovery descriptor.
    pub fn start(configuration: RunConfiguration) -> anyhow::Result<Self> {
        Self::start_in(sessions_directory()?, configuration)
    }

    /// Start a session in an explicit registry directory. This is useful for
    /// embedding and tests; the CLI uses the per-user default location.
    pub fn start_in(directory: PathBuf, configuration: RunConfiguration) -> anyhow::Result<Self> {
        Self::start_inner(Some(directory), configuration)
    }

    /// Start resource sampling without registering a discoverable MCP control
    /// endpoint. The TUI uses this when MCP control is disabled.
    pub fn start_monitor_only(configuration: RunConfiguration) -> anyhow::Result<Self> {
        Self::start_inner(None, configuration)
    }

    fn start_inner(
        directory: Option<PathBuf>,
        configuration: RunConfiguration,
    ) -> anyhow::Result<Self> {
        let session_id = Uuid::new_v4();
        let token = Uuid::new_v4().to_string();
        let resource_config = crate::config::resource_monitor_config();
        let control_enabled = directory.is_some();
        let (descriptor_path, listener, endpoint) = if let Some(directory) = directory {
            fs::create_dir_all(&directory)
                .with_context(|| format!("creating {}", directory.display()))?;
            let descriptor_path = directory.join(format!("{session_id}.json"));
            let (listener, endpoint) = bind_control_listener()?;
            (descriptor_path, Some(listener), endpoint)
        } else {
            (PathBuf::new(), None, String::new())
        };
        let state = SessionState {
            run_id: 1,
            status: SessionStatus::Starting,
            last_error: None,
            app_process: None,
            app_url: None,
            resource_monitor: ResourceMonitorStatus {
                interval_ms: resource_config.sample_interval_ms,
                ..ResourceMonitorStatus::default()
            },
            next_seq: 1,
            logs: VecDeque::with_capacity(LOG_CAPACITY.min(1024)),
            commands: VecDeque::new(),
            pending_mutation: None,
        };
        let shared = Arc::new(SharedSession {
            session_id,
            configuration,
            resource_config,
            control_enabled,
            endpoint: Mutex::new(endpoint),
            token,
            descriptor_path,
            descriptor_write: Mutex::new(()),
            state: Mutex::new(state),
            wake: Condvar::new(),
            mutation: Mutex::new(()),
        });
        let handle = SessionHandle {
            shared: Arc::clone(&shared),
            run_id: None,
        };
        if control_enabled {
            handle
                .persist_descriptor_checked()
                .context("registering aimer run session")?;
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let join = if let Some(listener) = listener {
            let thread_shutdown = Arc::clone(&shutdown);
            let thread_shared = Arc::clone(&shared);
            match thread::Builder::new()
                .name("aimer-mcp-control".to_string())
                .spawn(move || serve(listener, thread_shared, thread_shutdown))
            {
                Ok(join) => Some(join),
                Err(error) => {
                    let _ = fs::remove_file(&shared.descriptor_path);
                    return Err(error).context("starting aimer MCP control endpoint");
                }
            }
        } else {
            None
        };

        let monitor_shutdown = Arc::clone(&shutdown);
        let monitor_handle = handle.clone();
        let monitor_join = match thread::Builder::new()
            .name("aimer-resource-monitor".to_string())
            .spawn(move || crate::resource_monitor::run_monitor(monitor_handle, monitor_shutdown))
        {
            Ok(join) => join,
            Err(error) => {
                shutdown.store(true, Ordering::Release);
                if control_enabled {
                    let endpoint = handle
                        .shared
                        .endpoint
                        .lock()
                        .expect("endpoint mutex poisoned")
                        .clone();
                    let _ = TcpStream::connect(endpoint);
                }
                if let Some(join) = join {
                    let _ = join.join();
                }
                if control_enabled {
                    let _ = fs::remove_file(&handle.shared.descriptor_path);
                }
                return Err(error).context("starting aimer resource monitor");
            }
        };

        Ok(Self {
            handle,
            shutdown,
            join,
            monitor_join: Some(monitor_join),
        })
    }

    /// Clone the handle used by the run lifecycle.
    #[inline]
    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }
}

impl Drop for SessionRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.handle.shared.wake.notify_all();
        if let Some(join) = self.monitor_join.take() {
            let _ = join.join();
        }
        if let Some(join) = self.join.take() {
            let endpoint = self
                .handle
                .shared
                .endpoint
                .lock()
                .expect("endpoint mutex poisoned")
                .clone();
            let _ = TcpStream::connect(endpoint);
            let _ = join.join();
        }
        if self.handle.shared.control_enabled {
            let _ = fs::remove_file(&self.handle.shared.descriptor_path);
        }
    }
}

/// Registry access for session discovery. The default path is the user's
/// aimer sessions directory; tests may supply an isolated directory.
#[derive(Clone, Debug)]
pub struct SessionRegistry {
    directory: PathBuf,
}

impl SessionRegistry {
    /// Creates a registry rooted at directory.
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// Returns the default per-user session registry.
    pub fn default_location() -> anyhow::Result<Self> {
        Ok(Self::new(sessions_directory()?))
    }

    /// Lists live sessions, removing a descriptor only after its endpoint
    /// fails token-authenticated liveness validation.
    pub fn live(&self, project: Option<&Path>) -> anyhow::Result<Vec<SessionDescriptor>> {
        let mut sessions = Vec::new();
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(sessions),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", self.directory.display()));
            }
        };
        let expected_project = project.map(normalize_project_path);
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let path = entry.path();
            let descriptor = match read_descriptor(&path) {
                Ok(descriptor) => descriptor,
                Err(_) => continue,
            };
            let Some(file_session_id) = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| Uuid::parse_str(stem).ok())
            else {
                continue;
            };
            if descriptor.session_id != file_session_id {
                continue;
            }
            if expected_project.as_ref().is_some_and(|expected| {
                normalize_project_path(&descriptor.project_root) != *expected
            }) {
                continue;
            }
            if ControlClient::new(descriptor.clone())
                .ping_with_retries()
                .is_ok()
            {
                sessions.push(descriptor);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        sessions.sort_by_key(|session| session.session_id);
        Ok(sessions)
    }

    /// Load a descriptor by exact session ID and validate its endpoint/token.
    pub fn attach(&self, session_id: Uuid) -> anyhow::Result<ControlClient> {
        let path = self.directory.join(format!("{session_id}.json"));
        let descriptor = read_descriptor(&path)
            .with_context(|| format!("session '{session_id}' was not found"))?;
        if descriptor.session_id != session_id {
            anyhow::bail!("session descriptor identity mismatch for '{session_id}'");
        }
        let client = ControlClient::new(descriptor);
        if let Err(error) = client.ping_with_retries() {
            let _ = fs::remove_file(path);
            return Err(anyhow!("session '{session_id}' is not reachable: {error}"));
        }
        Ok(client)
    }
}

/// A validated client for the private session control endpoint.
#[derive(Clone, Debug)]
pub struct ControlClient {
    descriptor: SessionDescriptor,
}

impl ControlClient {
    pub(crate) fn new(descriptor: SessionDescriptor) -> Self {
        Self { descriptor }
    }

    /// Returns the descriptor used for this attachment.
    #[inline]
    pub fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }

    /// Send a control request and return its JSON result.
    pub fn request(&self, operation: &str, payload: Value) -> anyhow::Result<Value> {
        self.request_with_timeout(operation, payload, REQUEST_TIMEOUT)
    }

    /// Send a control request, retrying a bounded number of times when the
    /// loopback connection itself fails transiently (reset, or closed before
    /// a response arrived). Genuine application-level error replies (e.g. a
    /// well-formed `"busy"` response) are returned immediately, unretried.
    fn request_with_timeout(
        &self,
        operation: &str,
        payload: Value,
        read_timeout: Duration,
    ) -> anyhow::Result<Value> {
        let mut last_error = None;
        for attempt in 0..CONTROL_REQUEST_RETRY_COUNT {
            match self.send_request(operation, payload.clone(), read_timeout) {
                Ok(value) => return Ok(value),
                Err(error) if is_transient_control_error(&error) => {
                    last_error = Some(error);
                    if attempt + 1 < CONTROL_REQUEST_RETRY_COUNT {
                        thread::sleep(CONTROL_REQUEST_RETRY_DELAY);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("aimer MCP control request failed")))
    }

    /// Perform a single control request/response round trip over a fresh
    /// connection.
    fn send_request(
        &self,
        operation: &str,
        payload: Value,
        read_timeout: Duration,
    ) -> anyhow::Result<Value> {
        let request_id = Uuid::new_v4().to_string();
        let request = json!({
            "request_id": request_id,
            "token": self.descriptor.token,
            "operation": operation,
            "payload": payload,
        });
        let address = self
            .descriptor
            .endpoint
            .parse()
            .context("invalid aimer MCP endpoint")?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))?;
        stream.set_read_timeout(Some(read_timeout))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        let mut response_line = String::new();
        BufReader::new(stream).read_line(&mut response_line)?;
        let response: WireResponse = serde_json::from_str(&response_line)
            .context("invalid response from aimer MCP control endpoint")?;
        if response.request_id != request_id {
            return Err(anyhow!("mismatched control response ID"));
        }
        if response.ok {
            Ok(response.result.unwrap_or(Value::Null))
        } else {
            Err(anyhow!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "control request failed".to_string())
            ))
        }
    }

    /// Validate endpoint liveness and the descriptor token.
    pub fn ping(&self) -> anyhow::Result<SessionSnapshot> {
        let value = self.request_with_timeout("ping", Value::Null, LIVENESS_TIMEOUT)?;
        Ok(serde_json::from_value(value).context("invalid session ping response")?)
    }

    fn ping_with_retries(&self) -> anyhow::Result<SessionSnapshot> {
        let mut last_error = None;
        for attempt in 0..LIVENESS_RETRY_COUNT {
            match self.ping() {
                Ok(snapshot) => return Ok(snapshot),
                Err(error) => {
                    last_error = Some(error);
                    if attempt + 1 < LIVENESS_RETRY_COUNT {
                        thread::sleep(LIVENESS_RETRY_DELAY);
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("session liveness validation failed")))
    }
}

/// Whether a control request error is a transport-level hiccup (the
/// connection was reset, closed, or yielded an empty/truncated response)
/// rather than a genuine application-level reply, so it is safe to retry.
fn is_transient_control_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| {
                matches!(
                    io_error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::TimedOut
                )
            })
            || cause.downcast_ref::<serde_json::Error>().is_some()
    })
}
