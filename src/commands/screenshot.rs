//! `aimer screenshot`: a picture of the window of the app a run session launched,
//! drawn by the app itself — nothing else on the screen.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use serde_json::json;
use uuid::Uuid;

/// Arguments of `aimer screenshot`.
#[derive(Debug, Args)]
pub struct ScreenshotArgs {
    /// Where to write the picture. Defaults to `aimer-screenshot.png` (`.jpg` with
    /// `--jpeg`) in the current directory.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Session to take it from (see `aimer sessions`). Defaults to the only live one.
    #[arg(long)]
    session: Option<Uuid>,
    /// Only consider sessions of this project when choosing the live one.
    #[arg(long, conflicts_with = "session")]
    project: Option<PathBuf>,
    /// The widest the picture may be, in pixels. Without it the picture is the
    /// window's logical width, so a point in it is a point `aimer input` can
    /// click; `0` keeps the real pixels.
    #[arg(long)]
    width: Option<u32>,
    /// Write a JPEG of this quality (1 to 100), much smaller, instead of a PNG.
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=100))]
    jpeg: Option<u8>,
}

/// Where the picture goes when `--output` is not given.
fn default_output(jpeg: bool) -> PathBuf {
    PathBuf::from(if jpeg {
        "aimer-screenshot.jpg"
    } else {
        "aimer-screenshot.png"
    })
}

/// Run `aimer screenshot`.
pub fn execute(args: &ScreenshotArgs) -> anyhow::Result<()> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| default_output(args.jpeg.is_some()));
    // The run session writes the file, from its own working directory.
    let output = if output.is_absolute() {
        output
    } else {
        std::env::current_dir()?.join(output)
    };
    let client = super::input::connect(args.session, args.project.clone())?;
    let reply = client
        .request(
            "screenshot",
            json!({ "width": args.width, "jpeg": args.jpeg, "save_to": output.to_string_lossy() }),
        )
        .context("taking the screenshot")?;
    println!(
        "{}  {}x{} px, scale {}, {} bytes",
        Path::new(reply["path"].as_str().unwrap_or("")).display(),
        reply["width"],
        reply["height"],
        reply["scale"],
        reply["bytes"],
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        shot: ScreenshotArgs,
    }

    #[test]
    fn it_writes_a_png_unless_asked_for_a_jpeg() {
        assert_eq!(default_output(false), PathBuf::from("aimer-screenshot.png"));
        assert_eq!(default_output(true), PathBuf::from("aimer-screenshot.jpg"));
    }

    #[test]
    fn the_options_are_read() {
        let cli = Cli::try_parse_from(["shot", "-o", "out.png", "--width", "800", "--jpeg", "70"])
            .unwrap();
        assert_eq!((cli.shot.width, cli.shot.jpeg), (Some(800), Some(70)));
        assert_eq!(cli.shot.output, Some(PathBuf::from("out.png")));
    }

    #[test]
    fn a_jpeg_quality_outside_one_to_a_hundred_is_refused() {
        for quality in ["0", "101", "x"] {
            assert!(
                Cli::try_parse_from(["shot", "--jpeg", quality]).is_err(),
                "{quality}"
            );
        }
    }

    #[test]
    fn a_session_and_a_project_are_alternatives() {
        let id = Uuid::from_u128(1).to_string();
        assert!(Cli::try_parse_from(["shot", "--session", &id, "--project", "/p"]).is_err());
    }
}
