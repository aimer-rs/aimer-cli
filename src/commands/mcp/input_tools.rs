//! Tools that drive the running app, shaped like the computer-use tools agents
//! already know: `left_click` with a `coordinate`, `scroll` with a direction and
//! an amount, `key` with `ctrl+a`, `type` with text. Touch gestures and a raw
//! escape hatch cover what a phone and the rest of the protocol need.
//!
//! Each tool builds its sequence with [`aimer_prendre::actions`] and sends it to
//! the attached run session, which owns the connection to the app.

use aimer_prendre::actions::{self, ActionError, Direction};
use aimer_prendre::{Command, PointerButton};

use super::*;

/// Wheel steps a `scroll` takes when the caller does not say.
const DEFAULT_SCROLL_AMOUNT: f32 = 3.0;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct PointArguments {
    /// `[x, y]` in logical pixels, measured from the top-left corner of the
    /// app's window.
    coordinate: [f32; 2],
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ClickArguments {
    /// `[x, y]` to click at, in logical pixels from the window's top-left
    /// corner. The mouse glides there first. Omit it to click wherever the mouse
    /// already is: where the last click, move or scroll left it.
    #[serde(default)]
    coordinate: Option<[f32; 2]>,
}

impl ClickArguments {
    fn at(&self) -> Option<(f32, f32)> {
        self.coordinate.map(|c| (c[0], c[1]))
    }
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct DragArguments {
    /// `[x, y]` where the drag starts, in logical pixels from the window's top-left corner.
    start_coordinate: [f32; 2],
    /// `[x, y]` where the drag ends.
    coordinate: [f32; 2],
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct ScrollArguments {
    /// `[x, y]` to scroll at, in logical pixels from the window's top-left
    /// corner. The mouse glides there first. Omit it to scroll wherever the mouse
    /// already is.
    #[serde(default)]
    coordinate: Option<[f32; 2]>,
    /// Which way to scroll: up, down, left or right. Scrolling down shows what
    /// is below.
    scroll_direction: String,
    /// How many wheel steps to scroll. Defaults to 3.
    #[serde(default)]
    scroll_amount: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct PinchArguments {
    /// `[x, y]` the pinch is anchored at, in logical pixels from the window's top-left corner.
    coordinate: [f32; 2],
    /// The factor to change the scale by: above 1 zooms in, below 1 zooms out.
    scale: f32,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct TextArguments {
    /// The text to type, or for `key` a key or combination such as `Return`,
    /// `Escape`, `ctrl+a` or `cmd+shift+z`.
    text: String,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
struct ScreenshotArguments {
    /// The widest the picture may be, in pixels. Omit it for the window's logical
    /// width, which makes a point in the picture a point the input tools can
    /// click; `0` keeps the window's real pixels, which is larger.
    #[serde(default)]
    max_width: Option<u32>,
    /// A JPEG quality from 1 to 100. Omit it for a lossless PNG; a JPEG is much
    /// smaller for a busy window.
    #[serde(default)]
    jpeg_quality: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
struct RawInputArguments {
    /// Wire lines of the `aimer_prendre` protocol, sent in order, for example
    /// `back invoke`, `link aimer://item/7` or `file-drop /tmp/a.png`.
    commands: Vec<String>,
}

#[tool_router(router = input_tool_router, vis = "pub(super)")]
impl AimerMcpServer {
    #[tool(
        name = "left_click",
        description = "Click the left mouse button in the running Aimer app's window. With coordinate [x, y] the mouse glides there first; without it the click happens wherever the mouse already is (it must have been placed by an earlier click, mouse_move or scroll with a coordinate). Coordinates are logical pixels from the window's top-left corner. Needs an attached live session whose app accepts input."
    )]
    fn left_click(&self, Parameters(a): Parameters<ClickArguments>) -> CallToolResult {
        self.drive("left_click", actions::click(a.at(), PointerButton::Primary, 1))
    }

    #[tool(name = "right_click", description = "Click the right mouse button, at coordinate [x, y] or, without one, where the mouse already is.")]
    fn right_click(&self, Parameters(a): Parameters<ClickArguments>) -> CallToolResult {
        self.drive("right_click", actions::click(a.at(), PointerButton::Secondary, 1))
    }

    #[tool(name = "middle_click", description = "Click the middle mouse button, at coordinate [x, y] or, without one, where the mouse already is.")]
    fn middle_click(&self, Parameters(a): Parameters<ClickArguments>) -> CallToolResult {
        self.drive("middle_click", actions::click(a.at(), PointerButton::Middle, 1))
    }

    #[tool(name = "double_click", description = "Double-click the left mouse button, at coordinate [x, y] or, without one, where the mouse already is.")]
    fn double_click(&self, Parameters(a): Parameters<ClickArguments>) -> CallToolResult {
        self.drive("double_click", actions::click(a.at(), PointerButton::Primary, 2))
    }

    #[tool(name = "triple_click", description = "Triple-click the left mouse button, at coordinate [x, y] or, without one, where the mouse already is.")]
    fn triple_click(&self, Parameters(a): Parameters<ClickArguments>) -> CallToolResult {
        self.drive("triple_click", actions::click(a.at(), PointerButton::Primary, 3))
    }

    #[tool(name = "mouse_move", description = "Glide the mouse pointer to [x, y] in the app's window along a path, as a real mouse would, so what lies on the way sees it pass; use it to hover. The pointer starts from where the last injected mouse event left it.")]
    fn mouse_move(&self, Parameters(a): Parameters<PointArguments>) -> CallToolResult {
        self.drive("mouse_move", actions::mouse_move(a.coordinate[0], a.coordinate[1]))
    }

    #[tool(
        name = "left_click_drag",
        description = "Glide the mouse to start_coordinate, press the left button, glide to coordinate, and release."
    )]
    fn left_click_drag(&self, Parameters(a): Parameters<DragArguments>) -> CallToolResult {
        let [sx, sy] = a.start_coordinate;
        let [x, y] = a.coordinate;
        self.drive("left_click_drag", actions::drag((sx, sy), (x, y)))
    }

    #[tool(
        name = "scroll",
        description = "Scroll smoothly, as a trackpad swipe does, by scroll_amount steps of 40 pixels (default 3) in scroll_direction: up, down, left or right. With coordinate the mouse glides there first; without it the scroll happens wherever the mouse already is."
    )]
    fn scroll(&self, Parameters(a): Parameters<ScrollArguments>) -> CallToolResult {
        let commands = a.scroll_direction.parse::<Direction>().and_then(|direction| {
            actions::scroll(
                a.coordinate.map(|c| (c[0], c[1])),
                direction,
                a.scroll_amount.unwrap_or(DEFAULT_SCROLL_AMOUNT),
            )
        });
        self.drive("scroll", commands)
    }

    #[tool(
        name = "key",
        description = "Press and release a key or combination in the app: Return, Escape, Tab, BackSpace, Delete, Up, Down, Left, Right, Page_Up, Page_Down, Home, End, F1-F12, or modifiers joined with +, such as ctrl+a, cmd+c or ctrl+shift+z. Modifiers: ctrl, shift, alt, meta (cmd). A bare character is typed."
    )]
    fn key(&self, Parameters(a): Parameters<TextArguments>) -> CallToolResult {
        self.drive("key", actions::key(&a.text))
    }

    #[tool(
        name = "type",
        description = "Type text into the focused field one keystroke at a time, at an uneven human pace (a long text is typed faster, within about three seconds). Use `key` for Return, Tab and shortcuts."
    )]
    fn type_text(&self, Parameters(a): Parameters<TextArguments>) -> CallToolResult {
        self.drive("type", Ok(vec![actions::typing(&a.text)]))
    }

    #[tool(name = "tap", description = "Touch the screen with a finger at [x, y] and lift it, for an app on a phone or tablet.")]
    fn tap(&self, Parameters(a): Parameters<PointArguments>) -> CallToolResult {
        self.drive("tap", actions::tap(a.coordinate[0], a.coordinate[1]))
    }

    #[tool(
        name = "swipe",
        description = "Touch the screen at start_coordinate, slide one finger to coordinate, and lift it. Swipe up to scroll down."
    )]
    fn swipe(&self, Parameters(a): Parameters<DragArguments>) -> CallToolResult {
        let [sx, sy] = a.start_coordinate;
        let [x, y] = a.coordinate;
        self.drive("swipe", actions::swipe((sx, sy), (x, y)))
    }

    #[tool(
        name = "pinch",
        description = "Pinch at [x, y], multiplying the scale by `scale`: 1.5 zooms in by half, 0.5 zooms out by half."
    )]
    fn pinch(&self, Parameters(a): Parameters<PinchArguments>) -> CallToolResult {
        self.drive("pinch", actions::pinch(a.coordinate[0], a.coordinate[1], a.scale))
    }

    #[tool(
        name = "screenshot",
        description = "Take a screenshot of the running app's window, as the app itself drew it: only the app, nothing else on the screen. Returns the picture and its size. By default a pixel in the picture is a logical pixel, so its coordinates can be passed straight to the click, scroll and drag tools; if max_width makes it smaller, the text says what to divide by. Use it to see the result of an action before the next one."
    )]
    fn screenshot(&self, Parameters(a): Parameters<ScreenshotArguments>) -> CallToolResult {
        let reply = self.request_live("screenshot", json!({ "width": a.max_width, "jpeg": a.jpeg_quality }));
        picture_result(reply)
    }

    #[tool(
        name = "send_input",
        description = "Send raw aimer_prendre protocol lines to the app, in order, for events the other input tools do not cover: `back start|progress P|invoke|cancel`, `link URL`, `menu undo|redo|cut|copy|paste|select-all`, `file-hover PATH`, `file-hover-cancel`, `file-drop PATH`, `preedit RANGE TEXT`, `soft-backspace`, `touch ...`, `pinch`, `rotate`, `pan`, `quit`. The whole request is checked before anything is sent."
    )]
    fn send_input(&self, Parameters(a): Parameters<RawInputArguments>) -> CallToolResult {
        encode("send_input", self.request_live("input", json!({ "commands": a.commands })))
    }
}

impl AimerMcpServer {
    /// Sends the sequence an action built to the attached session's app.
    fn drive(&self, tool: &str, built: Result<Vec<Command>, ActionError>) -> CallToolResult {
        let result = built
            .map_err(|error| anyhow!("{error}"))
            .and_then(|commands| self.request_live("input", json!({ "commands": wire_lines(&commands) })));
        encode(tool, result)
    }
}

/// The result of a `screenshot`: the picture, and a line saying how its pixels map
/// to the coordinates the input tools take.
fn picture_result(reply: anyhow::Result<Value>) -> CallToolResult {
    let value = match reply {
        Ok(value) => value,
        Err(error) => return encode("screenshot", Err(error)),
    };
    let (Some(data), Some(mime)) = (value["data"].as_str(), value["mime"].as_str()) else {
        return encode("screenshot", Err(anyhow!("the session returned no picture")));
    };
    let (width, height) = (value["width"].as_u64().unwrap_or(0), value["height"].as_u64().unwrap_or(0));
    let scale = value["scale"].as_f64().unwrap_or(1.0);
    let mapping = if (scale - 1.0).abs() < 0.01 {
        "one pixel in the picture is one logical pixel, so its coordinates can be used with the input tools as they are".to_owned()
    } else {
        format!("divide a coordinate in the picture by {scale:.3} to get the coordinate the input tools take")
    };
    CallToolResult::success(vec![
        ContentBlock::text(format!("Screenshot of the app window, {width} x {height} pixels: {mapping}.")),
        ContentBlock::image(data, mime),
    ])
}

/// The wire line of each command, which is what the session sends on.
fn wire_lines(commands: &[Command]) -> Vec<String> {
    commands.iter().map(ToString::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::mcp::access::Access;

    #[test]
    fn every_input_tool_controls_the_running_app() {
        let tools = AimerMcpServer::input_tool_router().list_all();
        assert!(tools.len() >= 14, "the computer-use set plus touch and raw input");
        for tool in tools {
            assert_eq!(access_class(&tool.name), Access::Session, "{}", tool.name);
        }
    }

    #[test]
    fn the_names_match_the_computer_use_vocabulary() {
        let names: Vec<String> = AimerMcpServer::input_tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        for expected in [
            "left_click",
            "right_click",
            "double_click",
            "mouse_move",
            "left_click_drag",
            "scroll",
            "key",
            "type",
        ] {
            assert!(names.iter().any(|name| name == expected), "missing {expected}");
        }
    }

    #[test]
    fn a_screenshot_is_an_image_block_and_a_line_on_how_to_read_it() {
        let reply = json!({ "mime": "image/png", "width": 1150, "height": 800, "scale": 1.0, "data": "iVBORw0KGgo=" });
        let result = picture_result(Ok(reply));
        assert_ne!(result.is_error, Some(true), "a success, not an error result");
        assert_eq!(result.content.len(), 2);
        let image = result.content[1].as_image().expect("the picture");
        assert_eq!((image.mime_type.as_str(), image.data.as_str()), ("image/png", "iVBORw0KGgo="));
        let note = result.content[0].as_text().expect("a note").text.clone();
        assert!(note.contains("1150 x 800") && note.contains("as they are"), "{note}");
    }

    #[test]
    fn a_smaller_picture_says_what_to_divide_by() {
        let reply = json!({ "mime": "image/jpeg", "width": 800, "height": 557, "scale": 0.6956, "data": "AA==" });
        let note = picture_result(Ok(reply)).content[0].as_text().expect("a note").text.clone();
        assert!(note.contains("divide") && note.contains("0.696"), "{note}");
    }

    #[test]
    fn a_session_that_could_not_take_one_reports_why_and_returns_no_image() {
        let result = picture_result(Err(anyhow!("no live Aimer session is attached")));
        assert!(result.content.iter().all(|c| c.as_image().is_none()));
    }

    #[test]
    fn a_click_is_sent_as_the_wire_lines_of_its_commands() {
        let click = actions::click(Some((10.0, 20.0)), PointerButton::Primary, 1).expect("valid");
        assert_eq!(wire_lines(&click), ["glide 10 20", "down 10 20 primary", "up 10 20 primary"]);
    }

    #[test]
    fn a_click_without_a_coordinate_is_where_the_mouse_is() {
        let args: ClickArguments = serde_json::from_value(json!({})).expect("the coordinate is optional");
        assert_eq!(args.at(), None);
        let args: ClickArguments = serde_json::from_value(json!({ "coordinate": [3, 4] })).expect("given");
        assert_eq!(args.at(), Some((3.0, 4.0)));
        let click = actions::click(None, PointerButton::Primary, 2).expect("valid");
        assert_eq!(wire_lines(&click), ["down primary", "up primary", "down primary", "up primary"]);
    }
}
