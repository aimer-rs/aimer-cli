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
    Endpoint::new(addr.parse().expect("addr"), token).expect("endpoint").announcement()
}

#[test]
fn the_launch_environment_switches_the_server_on_with_our_token() {
    let route = InputRoute::new(None);
    let vars = route.environment();
    assert!(vars.contains(&(env::ENABLE, "1".to_owned())));
    let token = vars.iter().find(|(k, _)| *k == env::TOKEN).map(|(_, v)| v.clone()).expect("a token");
    assert_eq!(token.len(), 32, "128 bits as hex");
    assert!(vars.iter().all(|(k, _)| *k != env::BIND), "an app on this machine stays on loopback");
    assert_ne!(token, InputRoute::new(None).environment()[1].1, "every run has its own token");
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
    assert_eq!(route.endpoint_in(&announcement("127.0.0.1:4001", "someone-elses")), None);
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
    let err = session.send_input(json!({ "commands": ["move 1 2", "teleport 3 4"] })).unwrap_err();
    assert!(err.contains("teleport 3 4") && err.contains("unknown command"), "{err}");
    assert!(session.send_input(json!({ "commands": [] })).is_err());
    assert!(session.send_input(json!({ "commands": vec!["leave"; MAX_COMMANDS + 1] })).is_err());
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
        assert!(std::time::Instant::now() < deadline, "the session never attached");
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
    let got: Vec<String> = received.try_iter().filter(|l| !l.starts_with("hello ")).collect();
    assert_eq!(got, ["move 5 6", "down 5 6 primary", "up 5 6 primary"]);
}

#[test]
fn the_session_attaches_under_its_own_name_and_keeps_one_connection() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, received) = fake_app(&route.token);
    announced(&session, &route, addr);

    // Two requests, one connection (the fake app accepts only one).
    session.send_input(json!({ "commands": ["move 1 2"] })).expect("first");
    session.send_input(json!({ "commands": ["move 3 4"] })).expect("second");
    let lines: Vec<String> = received.try_iter().collect();
    let hellos: Vec<&String> = lines.iter().filter(|l| l.starts_with("hello ")).collect();
    assert_eq!(hellos, [&format!("hello {} aimer-cli", route.token)], "named, and said once");
    assert_eq!(lines.iter().filter(|l| l.starts_with("move")).count(), 2);
}

#[test]
fn a_relaunch_lets_go_of_the_old_connection() {
    let session = handle();
    let route = InputRoute::new(None);
    let (addr, _received) = fake_app(&route.token);
    announced(&session, &route, addr);
    session.send_input(json!({ "commands": ["move 1 2"] })).expect("sent");
    assert!(session.shared.input.lock().unwrap().client.is_some(), "kept for the next request");
    session.prepare_input(InputRoute::new(None));
    assert!(session.shared.input.lock().unwrap().client.is_none(), "dropped: the old app sees us leave");
}
