//! The route by which a run session sends input to the app it launched.
//!
//! The launcher asks the app to accept input through its environment, learns the
//! address the app listens on from the app's own log, and keeps both here. The
//! token never leaves this process: an agent or `aimer input` talks to the
//! session, and the session talks to the app.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use aimer_prendre::{Client, Command, Endpoint, Role, env};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::SessionHandle;

/// The most commands one request may carry; a gesture is a dozen.
const MAX_COMMANDS: usize = 256;

/// How an app is told to accept input, and how this machine reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputRoute {
    token: String,
    /// The address that leads to the app, when it is not this machine's own
    /// loopback: a phone's tunnel address.
    ip: Option<IpAddr>,
}

impl InputRoute {
    /// A route with a fresh random token, reaching the app at `ip` or, for an app
    /// on this machine, on loopback.
    pub fn new(ip: Option<IpAddr>) -> Self {
        Self {
            token: Uuid::new_v4().simple().to_string(),
            ip,
        }
    }

    /// Whether the app must listen beyond loopback to be reached.
    #[inline]
    pub fn is_remote(&self) -> bool {
        self.ip.is_some()
    }

    /// The variables to launch the app with.
    pub fn environment(&self) -> Vec<(&'static str, String)> {
        let mut vars = vec![
            (env::ENABLE, "1".to_owned()),
            (env::TOKEN, self.token.clone()),
        ];
        if self.is_remote() {
            // A phone's loopback is not reachable from here; its tunnel is.
            vars.push((env::BIND, "::".to_owned()));
        }
        vars
    }

    /// The endpoint an app announced in `message`, if it is one of ours.
    ///
    /// An announcement with another token is not for this run — the app did not
    /// take our settings — and could not be used anyway.
    fn endpoint_in(&self, message: &str) -> Option<Endpoint> {
        let endpoint = Endpoint::from_announcement(message)?;
        if endpoint.token() != self.token {
            return None;
        }
        Some(
            self.ip
                .map_or_else(|| endpoint.clone(), |ip| endpoint.with_ip(ip)),
        )
    }
}

/// What the app is told the session's own connection is called.
///
/// That connection stays open for the whole run, so it says the app can be driven
/// and nothing about whether anyone is driving it: it is the launcher's, and the
/// app is told so. An agent that controls the app has a lease of its own.
const ATTACH_NAME: &str = "aimer-cli";

/// What a one-shot request — `aimer input`, `aimer screenshot` — is called to the
/// app while it counts as an agent.
const ONE_SHOT_NAME: &str = "cli";

/// How long a one-shot request goes on counting as an agent after it ends. It has
/// no process that stays, as an agent's MCP server does, so it is there for a
/// moment after each command, and a run of them is there throughout.
#[cfg(not(test))]
const LINGER: Duration = Duration::from_secs(4);
#[cfg(test)]
const LINGER: Duration = Duration::from_millis(300);

/// How often the lingering connections are looked at to see whether one is over.
const REAP_EVERY: Duration = Duration::from_millis(100);

/// The most agents that may hold the app at once; a runaway client is refused
/// beyond it.
const MAX_LEASES: usize = 16;

/// One agent that is remote-controlling the app, and the connection that says so.
struct Lease {
    id: u64,
    name: String,
    /// The agent's own connection to the app, if the app is reachable. Held so
    /// that letting go of the lease disconnects it.
    client: Option<Client>,
    /// When it ends on its own, for a one-shot request. An agent's lease has none:
    /// it lasts as long as the agent's process does.
    expires: Option<Instant>,
}

/// What a session knows about driving its app.
#[derive(Default)]
pub(super) struct InputState {
    route: Option<InputRoute>,
    endpoint: Option<Endpoint>,
    /// The connection the app shows as attached. Kept open for the whole run, so
    /// the app says it is being driven from the moment it announces itself and
    /// not only while a request is in flight; requests reuse it.
    client: Option<Client>,
    /// Whether the app has been reached since it announced itself: a connection
    /// was made, or a request got through. Unlike `client`, which is out of its
    /// place while a request is using it, this does not flicker.
    reached: bool,
    /// The agents controlling the app. They outlive the app they were connected
    /// to: a relaunch reconnects them.
    leases: Vec<Lease>,
    next_lease: u64,
    /// Whether a thread is watching for one-shot requests to end.
    reaping: bool,
}

impl InputState {
    /// Whether the app announced itself and has been reached, so input tools can
    /// be offered. A launched app that has not announced, one that cannot be
    /// reached, and a session whose target cannot be driven are all not.
    pub(super) fn is_connected(&self) -> bool {
        self.endpoint.is_some() && self.reached
    }
}

#[derive(Deserialize)]
struct InputRequest {
    commands: Vec<String>,
}

/// What a `screenshot` asks for.
#[derive(Deserialize, Default)]
struct ScreenshotRequest {
    /// The widest the picture may be; absent is the window's logical width, `0`
    /// the real pixels.
    #[serde(default)]
    width: Option<u32>,
    /// A JPEG quality from 1 to 100; absent is a lossless PNG.
    #[serde(default)]
    jpeg: Option<u8>,
    /// Write the picture here and leave it out of the reply.
    #[serde(default)]
    save_to: Option<String>,
}

impl SessionHandle {
    /// Records how the app about to be launched will be reached. Forgets the
    /// endpoint of the previous launch: a restarted app listens somewhere new.
    pub fn prepare_input(&self, route: InputRoute) {
        let mut input = self.shared.input.lock().expect("input mutex poisoned");
        input.route = Some(route);
        input.endpoint = None;
        input.reached = false;
        // Dropping the connection tells the old app, if it is still there, that
        // this session has let go. The agents stay: they reconnect to the new one.
        input.client = None;
        for lease in &mut input.leases {
            lease.client = None;
        }
    }

    /// Looks for the app's announcement in one of its log lines.
    ///
    /// When it is there, attaches to the app on a thread of its own: this runs
    /// where the app's output is read, and reaching a phone can take a while.
    pub(crate) fn observe_app_message(&self, message: &str) {
        let endpoint = {
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            let Some(endpoint) = input
                .route
                .as_ref()
                .and_then(|route| route.endpoint_in(message))
            else {
                return;
            };
            input.endpoint = Some(endpoint.clone());
            input.client = None;
            input.reached = false;
            endpoint
        };
        let session = self.clone();
        let _ = std::thread::Builder::new()
            .name("aimer-input-attach".into())
            .spawn(move || session.attach(&endpoint));
    }

    /// Opens the named connection to `endpoint` and keeps it, unless the app
    /// has been relaunched elsewhere in the meantime.
    fn attach(&self, endpoint: &Endpoint) {
        let Ok(client) = Client::connect_with(endpoint, ATTACH_NAME, Role::Launcher) else {
            // A request connects for itself and says why it could not.
            return;
        };
        {
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            if input.endpoint.as_ref() == Some(endpoint) && input.client.is_none() {
                input.client = Some(client);
                input.reached = true;
            }
        }
        // Whoever was controlling the app before it announced itself is now in reach.
        self.connect_leases();
    }

    /// Says the agent called `name` is remote-controlling the app, until the
    /// returned lease is dropped.
    ///
    /// The app is told over a connection of its own, in the agent role, so it
    /// shows the agent is there for as long as the lease is held, busy or idle.
    /// A lease taken before the app is reachable connects when it is.
    pub(crate) fn open_lease(&self, name: &str) -> LeaseGuard {
        let id = {
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            let id = input.next_lease;
            input.next_lease += 1;
            if input
                .leases
                .iter()
                .filter(|lease| lease.expires.is_none())
                .count()
                < MAX_LEASES
            {
                input.leases.push(Lease {
                    id,
                    name: lease_name(name),
                    client: None,
                    expires: None,
                });
            }
            id
        };
        self.connect_leases();
        LeaseGuard {
            session: self.clone(),
            id,
        }
    }

    /// Counts a one-shot request as an agent for the next [`LINGER`], unless an
    /// agent that stays is already there.
    ///
    /// An agent's own commands arrive as requests too, and it has a lease that says
    /// it is there for as long as it is: counting them as well would leave a
    /// short-lived second agent behind, which keeps the app looking driven for a few
    /// seconds after the real one has gone.
    fn linger(&self) {
        {
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            if input.leases.iter().any(|lease| lease.expires.is_none()) {
                return;
            }
            let until = Some(Instant::now() + LINGER);
            match input
                .leases
                .iter_mut()
                .find(|lease| lease.expires.is_some())
            {
                Some(lease) => lease.expires = until,
                None => {
                    let id = input.next_lease;
                    input.next_lease += 1;
                    input.leases.push(Lease {
                        id,
                        name: ONE_SHOT_NAME.to_owned(),
                        client: None,
                        expires: until,
                    });
                }
            }
            if !input.reaping {
                input.reaping = true;
                let session = self.clone();
                let _ = std::thread::Builder::new()
                    .name("aimer-input-linger".into())
                    .spawn(move || session.reap());
            }
        }
        self.connect_leases();
    }

    /// Ends one-shot requests as their time runs out, and stops when none is left.
    fn reap(&self) {
        loop {
            std::thread::sleep(REAP_EVERY);
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            let now = Instant::now();
            input
                .leases
                .retain(|lease| lease.expires.is_none_or(|until| until > now));
            if input.leases.iter().all(|lease| lease.expires.is_none()) {
                input.reaping = false;
                return;
            }
        }
    }

    /// Opens the connection of every lease that has none and can have one.
    fn connect_leases(&self) {
        let (endpoint, waiting) = {
            let input = self.shared.input.lock().expect("input mutex poisoned");
            let Some(endpoint) = input.endpoint.clone() else {
                return;
            };
            let waiting: Vec<(u64, String)> = input
                .leases
                .iter()
                .filter(|lease| lease.client.is_none())
                .map(|lease| (lease.id, lease.name.clone()))
                .collect();
            (endpoint, waiting)
        };
        for (id, name) in waiting {
            // Reaching a phone can take a while, so no lock is held while it does.
            let Ok(client) = Client::connect_with(&endpoint, &name, Role::Agent) else {
                continue;
            };
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            // Still wanted, still the same app, and not already connected meanwhile.
            if input.endpoint.as_ref() == Some(&endpoint)
                && let Some(lease) = input
                    .leases
                    .iter_mut()
                    .find(|lease| lease.id == id && lease.client.is_none())
            {
                lease.client = Some(client);
            }
        }
    }

    fn release_lease(&self, id: u64) {
        self.shared
            .input
            .lock()
            .expect("input mutex poisoned")
            .leases
            .retain(|lease| lease.id != id);
    }

    /// Sends the wire lines in `payload` (`{"commands": [...]}`) to the app.
    ///
    /// Every line is read before the first is sent, so a typo does not leave a
    /// gesture half done.
    pub(super) fn send_input(&self, payload: Value) -> Result<Value, String> {
        let request: InputRequest = serde_json::from_value(payload).map_err(|e| e.to_string())?;
        if request.commands.is_empty() || request.commands.len() > MAX_COMMANDS {
            return Err(format!("send between 1 and {MAX_COMMANDS} commands"));
        }
        let commands = request
            .commands
            .iter()
            .map(|line| {
                line.parse::<Command>()
                    .map_err(|e| format!("`{line}`: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.linger();
        let (endpoint, mut client) = self.checkout()?;
        let mut sent = Ok(());
        for command in &commands {
            sent = client
                .send(command)
                .map_err(|e| format!("the app refused `{command}`: {e}"));
            if sent.is_err() {
                break;
            }
        }
        if sent.is_ok() {
            self.checkin(&endpoint, client);
        }
        sent?;
        Ok(json!({ "sent": commands.len() }))
    }

    /// Takes a picture of the app's window (`{"width", "jpeg", "save_to"}`, all
    /// optional) and returns it, base64 encoded, or writes it to `save_to`.
    ///
    /// The app captures, shrinks and encodes it on a thread of its own; this only
    /// carries the bytes. A picture asked to be written is not put in the reply,
    /// which keeps a large one out of the control channel.
    pub(super) fn screenshot(&self, payload: Value) -> Result<Value, String> {
        let request: ScreenshotRequest = if payload.is_null() {
            ScreenshotRequest::default()
        } else {
            serde_json::from_value(payload).map_err(|e| e.to_string())?
        };
        self.linger();
        let (endpoint, mut client) = self.checkout()?;
        let shot = client
            .screenshot(request.width, request.jpeg)
            .map_err(|e| format!("could not take a screenshot: {e}"))?;
        self.checkin(&endpoint, client);

        let mut reply = json!({
            "mime": shot.mime,
            "width": shot.width,
            "height": shot.height,
            "scale": shot.scale,
            "bytes": shot.bytes.len(),
        });
        match request.save_to {
            Some(path) => {
                std::fs::write(&path, &shot.bytes)
                    .map_err(|e| format!("could not write {path}: {e}"))?;
                reply["path"] = json!(path);
            }
            None => reply["data"] = json!(super::base64::encode(&shot.bytes)),
        }
        Ok(reply)
    }

    /// The connection to the app, taken out for one request: it is out while it is
    /// used, so a slow app does not hold up the thread that reads its output. If
    /// it has died, a new one is made. Hand it back with [`Self::checkin`].
    fn checkout(&self) -> Result<(Endpoint, Client), String> {
        let (endpoint, kept) = {
            let mut input = self.shared.input.lock().expect("input mutex poisoned");
            match (&input.route, &input.endpoint) {
                (None, _) => {
                    return Err(
                        "this run does not accept input: its target cannot be driven, or \
                                AIMER_PRENDRE=0 is set"
                            .to_owned(),
                    );
                }
                (Some(_), None) => {
                    return Err(
                        "the app has not announced its input server: it is still starting, \
                                it is a release build, or it does not start `aimer_prendre`"
                            .to_owned(),
                    );
                }
                (Some(_), Some(endpoint)) => (endpoint.clone(), input.client.take()),
            }
        };
        let client = match kept {
            Some(client) => client,
            // The token stays out of the message: it is the only secret in the system.
            None => Client::connect_with(&endpoint, ATTACH_NAME, Role::Launcher).map_err(|e| {
                self.note_reached(&endpoint, false);
                format!(
                    "could not reach the app's input server at {}: {e}",
                    endpoint.addr()
                )
            })?,
        };
        Ok((endpoint, client))
    }

    /// Puts the connection back in its place, unless the app was relaunched while
    /// it was out.
    fn checkin(&self, endpoint: &Endpoint, client: Client) {
        let mut input = self.shared.input.lock().expect("input mutex poisoned");
        if input.endpoint.as_ref() == Some(endpoint) && input.client.is_none() {
            input.client = Some(client);
            input.reached = true;
        }
    }

    /// Records whether the app at `endpoint` could be reached, unless it has been
    /// relaunched elsewhere in the meantime.
    fn note_reached(&self, endpoint: &Endpoint, reached: bool) {
        let mut input = self.shared.input.lock().expect("input mutex poisoned");
        if input.endpoint.as_ref() == Some(endpoint) {
            input.reached = reached;
        }
    }
}

/// An agent that is remote-controlling the app, for as long as this is held. See
/// [`SessionHandle::open_lease`].
pub(crate) struct LeaseGuard {
    session: SessionHandle,
    id: u64,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        self.session.release_lease(self.id);
    }
}

/// `raw` as a name the app will accept: plain characters only, and short.
/// Whatever asks for a lease picks its own name, so it is cleaned here.
fn lease_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._:/@+-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if name.is_empty() {
        "agent".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests;
