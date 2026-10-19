use super::*;

fn configuration(project_root: &Path) -> RunConfiguration {
    RunConfiguration {
        project_root: project_root.to_path_buf(),
        target: "macos".to_string(),
        device: Some("local".to_string()),
        execution_policy: "debug/native-aot/disabled".to_string(),
        parent_pid: 42,
    }
}

fn in_memory_handle(project_root: &Path) -> SessionHandle {
    let resource_config = ResourceMonitorConfig::default();
    let shared = Arc::new(SharedSession {
        session_id: Uuid::new_v4(),
        configuration: configuration(project_root),
        resource_config,
        control_enabled: false,
        endpoint: Mutex::new(String::new()),
        token: Uuid::new_v4().to_string(),
        descriptor_path: PathBuf::new(),
        descriptor_write: Mutex::new(()),
        state: Mutex::new(SessionState {
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
            logs: VecDeque::new(),
            commands: VecDeque::new(),
            pending_mutation: None,
        }),
        wake: Condvar::new(),
        mutation: Mutex::new(()),
    });
    SessionHandle {
        shared,
        run_id: None,
    }
}

#[test]
fn host_app_identity_is_visual_and_serializes_stable_names() {
    let identity = ProcessIdentity::host_app(
        19_999,
        Some(PathBuf::from(
            "/Applications/Jaime.app/Contents/MacOS/Jaime",
        )),
        None,
    );
    assert!(identity.is_host_visual_process());
    let value = serde_json::to_value(identity).unwrap();
    assert_eq!(value["pid"], 19_999);
    assert_eq!(value["scope"], "host");
    assert_eq!(value["role"], "app");
    assert_eq!(value["visual_target"], "host_process");
}

#[test]
fn device_and_web_identities_are_not_host_visual_processes() {
    assert!(
        !ProcessIdentity::device_app(Some(123), Some("com.example.app".into()))
            .is_host_visual_process()
    );
    assert!(!ProcessIdentity::web_runner(Some(123), None).is_host_visual_process());
    assert_eq!(
        ProcessIdentity::device_app(Some(123), None).visual_target,
        VisualTarget::DeviceApp
    );
}

#[test]
fn logs_paginate_by_monotonic_cursor_and_filter_by_run() {
    let temp = tempfile::tempdir().unwrap();
    let runtime =
        SessionRuntime::start_in(temp.path().join("sessions"), configuration(temp.path())).unwrap();
    let handle = runtime.handle();
    handle.push_log(LogStream::Build, "info", "compile one", None);
    handle.push_log(
        LogStream::App,
        "error",
        "panic",
        Some("src/main.rs:4".into()),
    );
    handle.set_status(SessionStatus::Running, None);
    let first = handle.get_logs(GetLogsRequest {
        limit: 1,
        ..GetLogsRequest::default()
    });
    assert_eq!(first.entries.len(), 1);
    let second = handle.get_logs(GetLogsRequest {
        cursor: first.next_cursor,
        stream: Some(LogStream::App),
        ..GetLogsRequest::default()
    });
    assert_eq!(second.entries[0].message, "panic");
    assert_eq!(second.entries[0].location.as_deref(), Some("src/main.rs:4"));
}

#[test]
fn restart_clears_pid_increments_run_and_preserves_logs() {
    let temp = tempfile::tempdir().unwrap();
    let runtime =
        SessionRuntime::start_in(temp.path().join("sessions"), configuration(temp.path())).unwrap();
    let handle = runtime.handle();
    handle.set_process(Some(ProcessIdentity::host_app(111, None, None)), None);
    handle.push_log(LogStream::App, "info", "before", None);
    let (reply, receiver) = mpsc::channel();
    handle.accept_restart(reply).unwrap();
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.run_id, 2);
    assert_eq!(snapshot.status, SessionStatus::Restarting);
    assert_eq!(snapshot.app_process, None);
    assert_eq!(handle.get_logs(GetLogsRequest::default()).entries.len(), 1);
    handle.set_status(SessionStatus::Running, None);
    let result = receiver.recv().unwrap().unwrap();
    assert_eq!(result.run_id, 2);
    assert_eq!(result.status, SessionStatus::Running);
}

#[test]
fn monitor_only_runtime_keeps_tui_state_without_registering_mcp_control() {
    let temp = tempfile::tempdir().unwrap();
    let configuration = configuration(temp.path());
    let runtime = SessionRuntime::start_monitor_only(configuration).unwrap();
    let handle = runtime.handle();

    assert_eq!(handle.snapshot().status, SessionStatus::Starting);
    assert!(
        SessionRegistry::new(temp.path().join("sessions"))
            .live(None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn registry_discovers_multiple_live_sessions_without_guessing() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let registry_dir = temp.path().join("sessions");
    let _first = SessionRuntime::start_in(registry_dir.clone(), configuration(&project)).unwrap();
    let _second = SessionRuntime::start_in(registry_dir.clone(), configuration(&project)).unwrap();
    let live = SessionRegistry::new(registry_dir)
        .live(Some(&project))
        .unwrap();
    assert_eq!(live.len(), 2);
}

#[test]
fn attach_validates_the_token_and_returns_the_current_app_identity() {
    let temp = tempfile::tempdir().unwrap();
    let registry_dir = temp.path().join("sessions");
    let runtime =
        SessionRuntime::start_in(registry_dir.clone(), configuration(temp.path())).unwrap();
    let handle = runtime.handle();
    handle.set_process(
        Some(ProcessIdentity::host_app(
            19_999,
            Some(PathBuf::from("/tmp/Jaime")),
            None,
        )),
        None,
    );
    handle.set_status(SessionStatus::Running, None);

    let client = SessionRegistry::new(registry_dir)
        .attach(handle.session_id())
        .unwrap();
    let snapshot = client.ping().unwrap();
    assert_eq!(
        snapshot
            .app_process
            .as_ref()
            .and_then(|process| process.pid),
        Some(19_999)
    );

    let mut bad_descriptor = client.descriptor().clone();
    bad_descriptor.token = "wrong-token".to_string();
    let bad_client = ControlClient::new(bad_descriptor);
    assert!(bad_client.ping().is_err());
}

#[test]
fn resource_monitor_interval_can_be_changed_and_restored_over_the_control_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let handle = in_memory_handle(temp.path());
    let configured_interval = handle.snapshot().resource_monitor.interval_ms;

    let changed = handle
        .request_status(
            "set_resource_interval",
            serde_json::json!({ "interval_ms": 500 }),
        )
        .unwrap();
    assert_eq!(changed["interval_ms"], 500);
    let live = handle
        .request_status("resource_usage", serde_json::Value::Null)
        .unwrap();
    assert_eq!(live["interval_ms"], 500);

    let restored = handle
        .request_status("set_resource_interval", serde_json::json!({}))
        .unwrap();
    assert_eq!(restored["interval_ms"], configured_interval);
    assert!(
        handle
            .request_status(
                "set_resource_interval",
                serde_json::json!({ "interval_ms": 499 })
            )
            .is_err()
    );
}

#[test]
fn console_restart_advances_run_and_rejects_stale_runner_updates() {
    let temp = tempfile::tempdir().unwrap();
    let handle = in_memory_handle(temp.path());
    handle.set_process(Some(ProcessIdentity::host_app(123, None, None)), None);
    handle.set_status(SessionStatus::Running, None);
    handle.publish_resource_sample(MonitorUpdate {
        run_id: 1,
        sample: crate::resource_monitor::ResourceSample {
            timestamp_unix_ms: 1,
            pid: Some(123),
            cpu_percent: Some(5.0),
            gpu_percent: None,
            memory_bytes: Some(1024),
            thread_count: Some(2),
            unavailable: Vec::new(),
        },
        recording: true,
    });
    let previous_runner = handle.scoped();

    assert_eq!(handle.begin_next_run().unwrap(), 2);
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.status, SessionStatus::Restarting);
    assert_eq!(snapshot.app_process, None);
    assert!(snapshot.resource_monitor.sample.is_none());
    assert!(!snapshot.resource_monitor.recording);

    previous_runner.set_process(Some(ProcessIdentity::host_app(456, None, None)), None);
    previous_runner.set_status(SessionStatus::Running, None);
    let snapshot = handle.snapshot();
    assert_eq!(snapshot.status, SessionStatus::Restarting);
    assert_eq!(snapshot.app_process, None);
}

#[test]
fn concurrent_readers_work_while_a_second_mutation_is_busy() {
    let temp = tempfile::tempdir().unwrap();
    let registry_dir = temp.path().join("sessions");
    let runtime =
        SessionRuntime::start_in(registry_dir.clone(), configuration(temp.path())).unwrap();
    let handle = runtime.handle();
    let client = SessionRegistry::new(registry_dir)
        .attach(handle.session_id())
        .unwrap();

    let first_client = client.clone();
    let first = thread::spawn(move || first_client.request("restart", Value::Null));
    let mut restart_reply = None;
    for _ in 0..100 {
        if let Some(SessionCommand::Restart { reply }) = handle.take_command() {
            restart_reply = Some(reply);
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let reply = restart_reply.expect("restart command should reach the run loop");
    handle.accept_restart(reply).unwrap();

    let status = client.ping().unwrap();
    assert_eq!(status.run_id, 2);
    let second = client.request("clear_logs", Value::Null).unwrap_err();
    assert_eq!(second.to_string(), "busy");

    handle.set_status(SessionStatus::Running, None);
    assert_eq!(first.join().unwrap().unwrap()["run_id"], 2);
}
