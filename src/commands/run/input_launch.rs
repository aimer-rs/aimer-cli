//! Asking the app a run launches to accept input.
//!
//! The app is told through its environment (see [`aimer_prendre::env`]); every
//! launcher passes the variables the way its platform carries them. The session
//! learns where the app listens from the app's own log, so nothing here waits.

use std::net::IpAddr;
use std::process::Command;

use aimer_prendre::env;
use serde_json::Value;
use uuid::Uuid;

use super::helpers::build_log;
use super::pipeline::RunContext;
use crate::session::InputRoute;

/// Whether the shell that started this run turned input off with
/// `AIMER_PRENDRE=0`, which also keeps the app from inheriting a switch.
fn opted_out() -> bool {
    std::env::var(env::ENABLE).is_ok_and(|value| value == "0")
}

/// Prepares the run's session to drive the app about to launch, and returns the
/// variables to launch it with. Empty when there is no session to drive it from,
/// or input is switched off.
///
/// `ip` is where this machine reaches the app when it is not on this machine.
pub(super) fn enable(ctx: &RunContext, ip: Option<IpAddr>) -> Vec<(&'static str, String)> {
    let Some(session) = &ctx.session else {
        return Vec::new();
    };
    if opted_out() {
        return Vec::new();
    }
    let route = InputRoute::new(ip);
    if route.is_remote() {
        build_log(
            &ctx.tx,
            "Input: the app listens beyond loopback so this machine can reach it; only a \
             random per-run token protects it. Set AIMER_PRENDRE=0 to turn input off.",
        );
    }
    let vars = route.environment();
    session.prepare_input(route);
    vars
}

/// The address at which this machine reaches a connected device, through the
/// tunnel `devicectl` sets up for it, or `None` when the device is not
/// connected or does not report one.
pub(super) fn device_address(device_id: &str) -> Option<IpAddr> {
    let path = std::env::temp_dir().join(format!("aimer-device-{}.json", Uuid::new_v4()));
    let status = Command::new("xcrun")
        .args(["devicectl", "device", "info", "details", "--device", device_id, "--json-output"])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let json = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    if !status.success() {
        return None;
    }
    tunnel_address(&json.ok()?)
}

/// Reads `result.connectionProperties.tunnelIPAddress` from the JSON that
/// `devicectl device info details` writes.
fn tunnel_address(json: &str) -> Option<IpAddr> {
    let value: Value = serde_json::from_str(json).ok()?;
    let address = value.pointer("/result/connectionProperties/tunnelIPAddress")?.as_str()?;
    // A scope id (`fd00::1%utun3`) is not part of the address.
    address.split('%').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_address_is_read_from_the_connection_properties() {
        let json = r#"{"info":{"outcome":"success"},"result":{"connectionProperties":{
            "transportType":"wired","tunnelIPAddress":"fd5c:7b1e:3a22::1","tunnelState":"connected"}}}"#;
        assert_eq!(tunnel_address(json), Some("fd5c:7b1e:3a22::1".parse().unwrap()));
    }

    #[test]
    fn a_scope_id_is_dropped_and_an_ipv4_address_is_accepted() {
        let with = |address: &str| {
            format!(r#"{{"result":{{"connectionProperties":{{"tunnelIPAddress":"{address}"}}}}}}"#)
        };
        assert_eq!(tunnel_address(&with("fe80::1%utun3")), Some("fe80::1".parse().unwrap()));
        assert_eq!(tunnel_address(&with("10.0.0.7")), Some("10.0.0.7".parse().unwrap()));
    }

    #[test]
    fn a_device_without_a_tunnel_has_no_address() {
        for json in [
            r#"{"result":{"connectionProperties":{"tunnelState":"disconnected"}}}"#,
            r#"{"result":{}}"#,
            r#"{"result":{"connectionProperties":{"tunnelIPAddress":"not an address"}}}"#,
            r#"{"result":{"connectionProperties":{"tunnelIPAddress":7}}}"#,
            "not json",
            "",
        ] {
            assert_eq!(tunnel_address(json), None, "{json}");
        }
    }
}
