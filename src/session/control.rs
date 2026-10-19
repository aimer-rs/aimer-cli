use super::*;

pub(super) fn bind_control_listener() -> anyhow::Result<(TcpListener, String)> {
    let mut last_error = None;
    for _ in 0..CONTROL_RETRY_COUNT {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => {
                listener.set_nonblocking(true)?;
                let endpoint = listener.local_addr()?.to_string();
                return Ok((listener, endpoint));
            }
            Err(error) => {
                last_error = Some(error);
                thread::sleep(CONTROL_RETRY_DELAY);
            }
        }
    }
    Err(anyhow!(
        "failed to bind aimer MCP endpoint: {}",
        last_error.map_or_else(|| "unknown error".to_string(), |e| e.to_string())
    ))
}

pub(super) fn serve(listener: TcpListener, shared: Arc<SharedSession>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let request_shared = Arc::clone(&shared);
                thread::spawn(move || serve_connection(stream, request_shared));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                tracing::debug!(error = %error, "aimer MCP endpoint stopped accepting connections");
                break;
            }
        }
    }
}

pub(super) fn serve_connection(mut stream: TcpStream, shared: Arc<SharedSession>) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let reader_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let response = match serde_json::from_str::<WireRequest>(&line) {
            Ok(request) => handle_request(request, &shared),
            Err(error) => WireResponse {
                request_id: String::new(),
                ok: false,
                result: None,
                error: Some(format!("invalid control request: {error}")),
            },
        };
        if serde_json::to_writer(&mut stream, &response).is_err()
            || stream.write_all(b"\n").is_err()
            || stream.flush().is_err()
        {
            break;
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

pub(super) fn handle_request(request: WireRequest, shared: &Arc<SharedSession>) -> WireResponse {
    if request.token != shared.token {
        return WireResponse {
            request_id: request.request_id,
            ok: false,
            result: None,
            error: Some("invalid session token".to_string()),
        };
    }
    let handle = SessionHandle {
        shared: Arc::clone(shared),
        run_id: None,
    };
    match handle.request_status(&request.operation, request.payload) {
        Ok(result) => WireResponse {
            request_id: request.request_id,
            ok: true,
            result: Some(result),
            error: None,
        },
        Err(error) => WireResponse {
            request_id: request.request_id,
            ok: false,
            result: None,
            error: Some(error),
        },
    }
}

pub(super) fn snapshot_locked(shared: &SharedSession, state: &SessionState) -> SessionSnapshot {
    SessionSnapshot {
        session_id: shared.session_id,
        run_id: state.run_id,
        project_root: shared.configuration.project_root.clone(),
        target: shared.configuration.target.clone(),
        device: shared.configuration.device.clone(),
        execution_policy: shared.configuration.execution_policy.clone(),
        parent_pid: shared.configuration.parent_pid,
        status: state.status,
        last_error: state.last_error.clone(),
        app_process: state.app_process.clone(),
        app_url: state.app_url.clone(),
        resource_monitor: state.resource_monitor.clone(),
    }
}

pub(super) fn logs_locked(state: &SessionState, request: GetLogsRequest) -> LogsPage {
    let limit = request.limit.clamp(1, 2_000);
    let stream = request.stream.unwrap_or(LogStream::All);
    let mut entries = Vec::with_capacity(limit);
    for entry in state.logs.iter().filter(|entry| {
        request.cursor.map_or(true, |cursor| entry.seq > cursor)
            && request.run_id.map_or(true, |run_id| entry.run_id == run_id)
            && stream.matches(entry.stream)
    }) {
        let mut entry = entry.clone();
        if !request.include_source_locations {
            entry.location = None;
        }
        entries.push(entry);
        if entries.len() == limit {
            break;
        }
    }
    let next_cursor = entries.last().map(|entry| entry.seq);
    LogsPage {
        entries,
        next_cursor,
        oldest_seq: state.logs.front().map(|entry| entry.seq),
        newest_seq: state.logs.back().map(|entry| entry.seq),
    }
}
