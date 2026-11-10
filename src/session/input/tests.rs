use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use super::*;
use crate::session::tests::in_memory_handle;

fn handle() -> SessionHandle {
    in_memory_handle(std::path::Path::new("/tmp/aimer-input-test"))
}

fn announcement(addr: &str, token: &str) -> String {
    Endpoint::new(addr.parse().expect("addr"), token)
        .expect("endpoint")
        .announcement()
}

#[test]
fn the_launch_environment_switches_the_server_on_with_our_token() {
    let route = InputRoute::new(None);
    let vars = route.environment();
    assert!(vars.contains(&(env::ENABLE, "1".to_owned())));
    let token = vars
        .iter()
        .find(|(k, _)| *k == env::TOKEN)
        .map(|(_, v)| v.clone())
        .expect("a token");
    assert_eq!(token.len(), 32, "128 bits as hex");
    assert!(
        vars.iter().all(|(k, _)| *k != env::BIND),
        "an app on this machine stays on loopback"
    );
    assert_ne!(
        token,
        InputRoute::new(None).environment()[1].1,
        "every run has its own token"
    );
}

#[test]
fn an_app_on_a_phone_is_asked_to_listen_beyond_loopback() {
    let route = InputRoute::new(Some("fd00::1".parse().expect("ip")));
    assert!(route.is_remote());
    assert!(route.environment().contains(&(env::BIND, "::".to_owned())));
}

#[test]
fn the_announcement_is_found_in_a_json_log_line_and_the_address_is_replaced() {
    let route = InputRoute::new(Some("fd00::1".parse().expect("ip")));
    let token = route.token.clone();
    // The app logs one JSON record per line; the message sits inside it.
    let line = format!(
        r#"{{"level":"info","message":"{}"}}"#,
        announcement("[::]:4001", &token)
    );
    let endpoint = route.endpoint_in(&line).expect("ours");
    assert_eq!(endpoint.addr(), "[fd00::1]:4001".parse().expect("addr"));
    assert_eq!(endpoint.token(), token);
}

#[test]
fn an_announcement_with_another_token_is_not_ours() {
    let route = InputRoute::new(None);
    assert_eq!(
        route.endpoint_in(&announcement("127.0.0.1:4001", "someone-elses")),
        None
    );
    assert_eq!(route.endpoint_in("window ready"), None);
}

#[test]
fn input_is_refused_with_a_reason_until_the_app_has_announced_itself() {
    let session = handle();
    let payload = json!({ "commands": ["move 1 2"] });
    let err = session.send_input(payload.clone()).unwrap_err();
    assert!(err.contains("does not accept input"), "{err}");

    session.prepare_input(InputRoute::new(None));
    let err = session.send_input(payload).unwrap_err();
    assert!(err.contains("has not announced"), "{err}");
}

#[test]
fn a_bad_line_stops_the_request_before_anything_is_sent() {
    let session = handle();
    session.prepare_input(InputRoute::new(None));
    let err = session
        .send_input(json!({ "commands": ["move 1 2", "teleport 3 4"] }))
        .unwrap_err();
    assert!(
        err.contains("teleport 3 4") && err.contains("unknown command"),
        "{err}"
    );
    assert!(session.send_input(json!({ "commands": [] })).is_err());
    assert!(
        session
            .send_input(json!({ "commands": vec!["leave"; MAX_COMMANDS + 1] }))
            .is_err()
    );
    assert!(session.send_input(json!({ "nonsense": true })).is_err());
}

#[test]
fn a_restart_forgets_the_previous_endpoint() {
    let session = handle();
    let route = InputRoute::new(None);
    session.prepare_input(route.clone());
    session.observe_app_message(&announcement("127.0.0.1:4001", &route.token));
    assert!(session.shared.input.lock().unwrap().endpoint.is_some());
    session.prepare_input(InputRoute::new(None));
    assert!(session.shared.input.lock().unwrap().endpoint.is_none());
}

#[test]
fn a_session_is_connected_to_its_app_only_once_it_has_reached_it() {
    let session = handle();
    assert!(!session.snapshot().input_connected, "nothing is launched");

    let route = InputRoute::new(None);
    session.prepare_input(route.clone());
    assert!(
        !session.snapshot().input_connected,
        "launched, but the app has not announced itself"
    );

    let (addr, _received) = fake_app(&route.token);
    announced(&session, &route, addr);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !session.snapshot().input_connected {
        assert!(
            std::time::Instant::now() < deadline,
            "the session never reported the app as connected"
        );
        thread::sleep(std::time::Duration::from_millis(5));
    }

    session.prepare_input(InputRoute::new(None));
    assert!(
        !session.snapshot().input_connected,
        "a restart forgets the old app"
    );
}

#[test]
fn an_announcement_nobody_answers_is_not_a_connection() {
    let session = handle();
    let route = InputRoute::new(None);
    // A port nothing listens on: bind and drop.
    let addr = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr");
    session.prepare_input(route.clone());
    session.observe_app_message(&announcement(&addr.to_string(), &route.token));
    thread::sleep(std::time::Duration::from_millis(300));
    assert!(!session.snapshot().input_connected);
}

/// A stand-in for the app's server: answers `ok` to a hello with the right
/// token and to each line, and reports every line it is sent, hello included.
fn fake_app(token: &str) -> (std::net::SocketAddr, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    let expected = format!("hello {token}");
    thread::spawn(move || {
        // One connection only: a second request must reuse it.
        let (stream, _) = listener.accept().expect("accept");
        let mut writer = stream.try_clone().expect("clone");
        for line in BufReader::new(stream).lines() {
            let line = line.expect("line");
            let reply = if !line.starts_with("hello ") || line.starts_with(&expected) {
                "ok\n"
            } else {
                "err unauthorized\n"
            };
            writer.write_all(reply.as_bytes()).expect("reply");
            tx.send(line).expect("report");
        }
    });
    (addr, rx)
}

/// Announces the app and waits for the session to attach to it, which it does on
/// a thread of its own.
fn announced(session: &SessionHandle, route: &InputRoute, addr: std::net::SocketAddr) {
    session.prepare_input(route.clone());
    session.observe_app_message(&announcement(&addr.to_string(), &route.token));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.shared.input.lock().unwrap().client.is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the session never attached"
        );
        thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn commands_reach_the_app_in_order_over_the_wire() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, received) = fake_app(&route.token);
    announced(&session, &route, addr);

    let result = session
        .send_input(json!({ "commands": ["move 5 6", "down 5 6 primary", "up 5 6 primary"] }))
        .expect("sent");
    assert_eq!(result["sent"], 3);
    let got: Vec<String> = received
        .try_iter()
        .filter(|l| !l.starts_with("hello "))
        .collect();
    assert_eq!(got, ["move 5 6", "down 5 6 primary", "up 5 6 primary"]);
}

#[test]
fn the_session_attaches_under_its_own_name_and_keeps_one_connection() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, received) = fake_app(&route.token);
    announced(&session, &route, addr);

    // Two requests, one connection (the fake app accepts only one).
    session
        .send_input(json!({ "commands": ["move 1 2"] }))
        .expect("first");
    session
        .send_input(json!({ "commands": ["move 3 4"] }))
        .expect("second");
    let lines: Vec<String> = received.try_iter().collect();
    let hellos: Vec<&String> = lines.iter().filter(|l| l.starts_with("hello ")).collect();
    assert_eq!(
        hellos,
        [&format!("hello {} aimer-cli role=launcher", route.token)],
        "named, and said once"
    );
    assert_eq!(lines.iter().filter(|l| l.starts_with("move")).count(), 2);
}

#[test]
fn a_relaunch_lets_go_of_the_old_connection() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, _received) = fake_app(&route.token);
    announced(&session, &route, addr);
    session
        .send_input(json!({ "commands": ["move 1 2"] }))
        .expect("sent");
    assert!(
        session.shared.input.lock().unwrap().client.is_some(),
        "kept for the next request"
    );
    session.prepare_input(InputRoute::new(None));
    assert!(
        session.shared.input.lock().unwrap().client.is_none(),
        "dropped: the old app sees us leave"
    );
}

// ---- who the app is told it is talking to

/// A stand-in for the app that takes any number of connections, answers `ok` to
/// every line, and reports `(connection number, line)` for each, with `<closed>`
/// as the line when a connection ends.
fn fake_app_many() -> (std::net::SocketAddr, mpsc::Receiver<(usize, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (number, stream) in listener.incoming().enumerate() {
            let stream = stream.expect("accept");
            let tx = tx.clone();
            thread::spawn(move || {
                let mut writer = stream.try_clone().expect("clone");
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    writer.write_all(b"ok\n").expect("reply");
                    tx.send((number, line)).expect("report");
                }
                let _ = tx.send((number, "<closed>".to_owned()));
            });
        }
    });
    (addr, rx)
}

fn next_line(rx: &mpsc::Receiver<(usize, String)>) -> (usize, String) {
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("the app heard something")
}

fn announce(session: &SessionHandle, route: &InputRoute, addr: std::net::SocketAddr) {
    session.prepare_input(route.clone());
    session.observe_app_message(&announcement(&addr.to_string(), &route.token));
}

#[test]
fn the_session_tells_the_app_it_is_the_launcher_not_an_agent() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, heard) = fake_app_many();
    announce(&session, &route, addr);
    let (_, hello) = next_line(&heard);
    assert_eq!(
        hello,
        format!("hello {} aimer-cli role=launcher", route.token)
    );
}

#[test]
fn an_agent_lease_is_an_agent_connection_for_as_long_as_it_is_held() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, heard) = fake_app_many();
    announce(&session, &route, addr);
    next_line(&heard); // the launcher

    let lease = session.open_lease("mcp");
    let (number, hello) = next_line(&heard);
    assert_eq!(
        hello,
        format!("hello {} mcp", route.token),
        "an agent says no role: it is the default"
    );
    assert_ne!(number, 0, "its own connection, not the launcher's");

    drop(lease);
    assert_eq!(
        next_line(&heard),
        (number, "<closed>".to_owned()),
        "letting go disconnects it"
    );
}

#[test]
fn a_lease_taken_before_the_app_announces_itself_connects_when_it_does() {
    let session = handle();
    let route = InputRoute::new(None);
    session.prepare_input(route.clone());
    let _lease = session.open_lease("mcp");

    let (addr, heard) = fake_app_many();
    session.observe_app_message(&announcement(&addr.to_string(), &route.token));
    let mut hellos = vec![next_line(&heard).1, next_line(&heard).1];
    hellos.sort();
    assert_eq!(
        hellos,
        [
            format!("hello {} aimer-cli role=launcher", route.token),
            format!("hello {} mcp", route.token)
        ]
    );
}

#[test]
fn a_relaunched_app_gets_its_agents_back() {
    let session = handle();
    let route = InputRoute::new(None);
    let (first, _) = fake_app_many();
    announce(&session, &route, first);
    let _lease = session.open_lease("mcp");

    let route = InputRoute::new(None);
    let (second, heard) = fake_app_many();
    announce(&session, &route, second);
    let mut hellos = vec![next_line(&heard).1, next_line(&heard).1];
    hellos.sort();
    assert!(
        hellos
            .iter()
            .any(|h| h == &format!("hello {} mcp", route.token)),
        "{hellos:?}"
    );
}

#[test]
fn a_one_shot_request_is_an_agent_for_a_moment_and_then_not() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, heard) = fake_app_many();
    announce(&session, &route, addr);
    next_line(&heard); // the launcher

    session
        .send_input(json!({ "commands": ["move 1 2"] }))
        .expect("sent");
    let mut lines = Vec::new();
    // The agent says hello, the command goes by the launcher's connection, and
    // after the linger the agent's connection closes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline
        && !lines.iter().any(|(_, l): &(usize, String)| l == "<closed>")
    {
        if let Ok(line) = heard.recv_timeout(std::time::Duration::from_millis(100)) {
            lines.push(line);
        }
    }
    assert!(
        lines
            .iter()
            .any(|(_, l)| l == &format!("hello {} cli", route.token)),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|(_, l)| l == "<closed>"),
        "it lingers and then leaves: {lines:?}"
    );
}

#[test]
fn a_lease_connection_stays_open_while_idle_and_ends_the_lease_when_the_client_goes() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let session = handle();
    // The real control listener is non-blocking, and a connection it accepts can
    // inherit that, which would make an idle read fail at once and drop the lease.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("non-blocking");
    let addr = listener.local_addr().expect("addr");
    let shutdown = Arc::new(AtomicBool::new(false));
    let (shared, flag) = (Arc::clone(&session.shared), Arc::clone(&shutdown));
    thread::spawn(move || crate::session::control::serve(listener, shared, flag));

    let mut stream = std::net::TcpStream::connect(addr).expect("connect");
    let request = json!({ "request_id": "r1", "token": session.shared.token, "operation": "remote_begin", "payload": { "name": "mcp:test" } });
    writeln!(stream, "{request}").expect("send");
    let mut reply = String::new();
    BufReader::new(stream.try_clone().expect("clone"))
        .read_line(&mut reply)
        .expect("reply");
    assert!(reply.contains("\"ok\":true"), "{reply}");

    // Idle for a while: the lease is still held.
    thread::sleep(std::time::Duration::from_millis(700));
    assert_eq!(
        session.shared.input.lock().unwrap().leases.len(),
        1,
        "an idle agent is still there"
    );

    // The client goes away, and so does the lease.
    drop(stream);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !session.shared.input.lock().unwrap().leases.is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the lease outlived its client"
        );
        thread::sleep(std::time::Duration::from_millis(20));
    }
    shutdown.store(true, Ordering::Release);
}

#[test]
fn a_request_adds_no_lingering_agent_while_a_real_one_holds_the_app() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, heard) = fake_app_many();
    announce(&session, &route, addr);
    next_line(&heard); // the launcher
    let lease = session.open_lease("mcp");
    next_line(&heard); // the agent

    // The agent's own commands arrive as requests too. It is already there, so they
    // must not leave a second, shorter-lived agent behind: that one would keep the
    // app looking driven for a few seconds after the real agent has gone.
    session
        .send_input(json!({ "commands": ["move 1 2"] }))
        .expect("sent");
    thread::sleep(std::time::Duration::from_millis(600));
    let mut lines = Vec::new();
    while let Ok(line) = heard.try_recv() {
        lines.push(line);
    }
    assert!(
        !lines.iter().any(|(_, l)| l.contains(" cli")),
        "no lingering agent: {lines:?}"
    );

    drop(lease);
    session
        .send_input(json!({ "commands": ["move 3 4"] }))
        .expect("sent");
    let hello = next_line(&heard);
    assert!(
        hello.1.contains("closed"),
        "with nobody holding the app, a one-shot request still counts: {hello:?}"
    );
}
