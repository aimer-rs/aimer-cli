//! `aimer input`: send a click, a key or a gesture to the app a run session
//! launched. The same vocabulary as the `left_click`, `key` and `scroll` MCP
//! tools, for a shell script or a person.

use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use aimer_prendre::actions::{self, ActionError};
use aimer_prendre::{Command, PointerButton};
use clap::{Args, Subcommand, ValueEnum};
use serde_json::json;
use uuid::Uuid;

use crate::session::{ControlClient, SessionRegistry};

/// Wheel steps a `scroll` takes when none are given.
const DEFAULT_SCROLL_AMOUNT: f32 = 3.0;

/// Arguments of `aimer input`.
#[derive(Debug, Args)]
pub struct InputArgs {
    /// Session to send to (see `aimer sessions`). Defaults to the only live one.
    #[arg(long)]
    session: Option<Uuid>,
    /// Only consider sessions of this project when choosing the live one.
    #[arg(long, conflicts_with = "session")]
    project: Option<PathBuf>,
    #[command(subcommand)]
    action: InputAction,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Button {
    Left,
    Right,
    Middle,
}

/// What to do in the app. Coordinates are logical pixels from the top-left
/// corner of its window.
///
/// Each variant that takes numbers allows negative ones itself: clap does not
/// pass that setting down from `aimer input` to its subcommands.
#[derive(Debug, Subcommand)]
enum InputAction {
    /// Click a mouse button, at a point the pointer glides to first, or without
    /// one where the pointer already is.
    #[command(allow_negative_numbers = true)]
    Click {
        #[arg(requires = "y")]
        x: Option<f32>,
        #[arg(requires = "x")]
        y: Option<f32>,
        #[arg(long, value_enum, default_value = "left")]
        button: Button,
        /// Presses: 2 is a double click, 3 a triple click.
        #[arg(long, default_value_t = 1)]
        count: u8,
    },
    /// Glide the pointer there along a path, for example to hover.
    #[command(allow_negative_numbers = true)]
    Move { x: f32, y: f32 },
    /// Glide to the first point, press, glide to the second, and release.
    #[command(allow_negative_numbers = true)]
    Drag { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// Scroll smoothly, as a trackpad does: up, down, left or right.
    #[command(allow_negative_numbers = true)]
    Scroll {
        direction: String,
        /// Steps of 40 pixels.
        #[arg(default_value_t = DEFAULT_SCROLL_AMOUNT)]
        amount: f32,
        /// Glide the mouse here first. Without it the scroll happens wherever the
        /// mouse already is.
        #[arg(long, num_args = 2, value_names = ["X", "Y"])]
        at: Option<Vec<f32>>,
    },
    /// Press a key or combination: Return, Escape, ctrl+a, cmd+shift+z, F5.
    Key { spec: String },
    /// Type text into the focused field a keystroke at a time, at a human pace.
    Type { text: String },
    /// Touch the screen at a point and lift.
    #[command(allow_negative_numbers = true)]
    Tap { x: f32, y: f32 },
    /// Slide one finger from the first point to the second.
    #[command(allow_negative_numbers = true)]
    Swipe { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// Pinch at a point, multiplying the scale: 1.5 zooms in, 0.5 zooms out.
    #[command(allow_negative_numbers = true)]
    Pinch { x: f32, y: f32, scale: f32 },
    /// Send raw `aimer_prendre` lines: `back invoke`, `link URL`, `file-drop PATH`.
    Send {
        #[arg(required = true)]
        lines: Vec<String>,
    },
}

impl InputAction {
    /// The wire lines this action sends.
    fn lines(&self) -> anyhow::Result<Vec<String>> {
        let built: Result<Vec<Command>, ActionError> = match self {
            Self::Click { x, y, button, count } => {
                let button = match button {
                    Button::Left => PointerButton::Primary,
                    Button::Right => PointerButton::Secondary,
                    Button::Middle => PointerButton::Middle,
                };
                actions::click(x.zip(*y), button, *count)
            }
            Self::Move { x, y } => actions::mouse_move(*x, *y),
            Self::Drag { x1, y1, x2, y2 } => actions::drag((*x1, *y1), (*x2, *y2)),
            Self::Scroll { direction, amount, at } => direction.parse().and_then(|direction| {
                actions::scroll(at.as_ref().map(|point| (point[0], point[1])), direction, *amount)
            }),
            Self::Key { spec } => actions::key(spec),
            Self::Type { text } => Ok(vec![actions::typing(text)]),
            Self::Tap { x, y } => actions::tap(*x, *y),
            Self::Swipe { x1, y1, x2, y2 } => actions::swipe((*x1, *y1), (*x2, *y2)),
            Self::Pinch { x, y, scale } => actions::pinch(*x, *y, *scale),
            Self::Send { lines } => return Ok(lines.clone()),
        };
        Ok(built.map_err(|error| anyhow!("{error}"))?.iter().map(ToString::to_string).collect())
    }
}

/// Run `aimer input`.
pub fn execute(args: &InputArgs) -> anyhow::Result<()> {
    let lines = args.action.lines()?;
    let client = connect(args.session, args.project.clone())?;
    let reply = client
        .request("input", json!({ "commands": lines }))
        .context("sending input to the app")?;
    println!("sent {}", reply["sent"]);
    Ok(())
}

pub(super) fn connect(session: Option<Uuid>, project: Option<PathBuf>) -> anyhow::Result<ControlClient> {
    let registry = SessionRegistry::default_location()?;
    if let Some(id) = session {
        return registry.attach(id);
    }
    let project = project.map(super::sessions::resolve_project_path).transpose()?;
    let sessions = registry.live(project.as_deref())?;
    let chosen = choose(sessions, |s| {
        format!("  {}  {}  {}", s.session_id, s.target, s.project_root.display())
    })?;
    Ok(ControlClient::new(chosen))
}

/// The one session to send to, or why there is not exactly one. `describe`
/// names a session for the list shown when there are several.
fn choose<T>(mut sessions: Vec<T>, describe: impl Fn(&T) -> String) -> anyhow::Result<T> {
    match sessions.len() {
        0 => bail!("no live Aimer run session; start the app with `aimer run`"),
        1 => Ok(sessions.remove(0)),
        _ => {
            let listing: Vec<String> = sessions.iter().map(describe).collect();
            bail!("several live sessions; choose one with --session:\n{}", listing.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        input: InputArgs,
    }

    fn lines(args: &[&str]) -> Vec<String> {
        let cli = Cli::try_parse_from(std::iter::once("input").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?}: {e}"));
        cli.input.action.lines().unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    #[test]
    fn a_click_is_the_wire_lines_of_a_click() {
        assert_eq!(lines(&["click", "10", "20"]), ["glide 10 20", "down 10 20 primary", "up 10 20 primary"]);
        assert_eq!(lines(&["click", "1", "2", "--button", "right"])[1], "down 1 2 secondary");
        assert_eq!(lines(&["click", "1", "2", "--count", "2"]).len(), 5);
    }

    #[test]
    fn a_click_without_a_point_is_where_the_pointer_already_is() {
        assert_eq!(lines(&["click"]), ["down primary", "up primary"]);
        assert_eq!(lines(&["click", "--button", "right", "--count", "2"]).len(), 4);
        // A point is both numbers or neither.
        assert!(Cli::try_parse_from(["input", "click", "5"]).is_err());
    }

    #[test]
    fn negative_coordinates_are_numbers_not_flags() {
        assert_eq!(lines(&["move", "-5", "10"]), ["glide -5 10"]);
    }

    #[test]
    fn a_scroll_is_a_smooth_trackpad_swipe_of_three_steps_by_default_and_glides_first_only_when_told_where() {
        // A step is 40 pixels, and down moves the content up.
        assert_eq!(lines(&["scroll", "down"]), ["trackpad 0 -120"], "where the mouse already is");
        assert_eq!(lines(&["scroll", "UP", "5"]), ["trackpad 0 200"]);
        assert_eq!(
            lines(&["scroll", "down", "2", "--at", "10", "-20"]),
            ["glide 10 -20", "settle", "trackpad 0 -80"]
        );
    }

    #[test]
    fn a_drag_is_a_glide_a_press_a_glide_and_a_release() {
        assert_eq!(
            lines(&["drag", "0", "0", "-10", "-20"]),
            ["glide 0 0", "down 0 0 primary", "glide -10 -20", "up -10 -20 primary"]
        );
    }

    #[test]
    fn keys_and_text_and_raw_lines_pass_through() {
        assert_eq!(lines(&["key", "ctrl+a"]), ["key press ctrl other:a", "key release ctrl other:a"]);
        assert_eq!(lines(&["type", "hello world"]), ["typing - hello world"]);
        assert_eq!(lines(&["send", "back invoke", "link aimer://x"]), ["back invoke", "link aimer://x"]);
    }

    #[test]
    fn touch_gestures_use_one_finger() {
        assert_eq!(lines(&["tap", "7", "8"]), ["touch start 1 7 8", "touch end 1 7 8"]);
        assert_eq!(lines(&["swipe", "0", "100", "0", "0"]).first().map(String::as_str), Some("touch start 1 0 100"));
    }

    #[test]
    fn a_bad_action_is_an_error_not_a_partial_send() {
        for args in [&["scroll", "sideways"][..], &["key", "ctrl+"], &["click", "1", "2", "--count", "9"]] {
            let cli = Cli::try_parse_from(std::iter::once("input").chain(args.iter().copied())).unwrap();
            assert!(cli.input.action.lines().is_err(), "{args:?}");
        }
    }

    #[test]
    fn exactly_one_live_session_is_chosen_and_anything_else_says_why() {
        let name = |s: &&str| format!("  {s}");
        assert_eq!(choose(vec!["only"], name).unwrap(), "only");
        assert!(choose(Vec::<&str>::new(), name).unwrap_err().to_string().contains("aimer run"));
        let many = choose(vec!["first", "second"], name).unwrap_err().to_string();
        assert!(many.contains("--session") && many.contains("first") && many.contains("second"), "{many}");
    }
}
