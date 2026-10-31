//! Building the `.dmg`: a compressed disk image holding the app next to an
//! `Applications` shortcut, in a window that tells the user to drag one onto the
//! other.
//!
//! The window is the one Finder shows when the image is opened, so it is
//! recorded by Finder itself: the image is first made writable and mounted, a
//! script lays the window out (background, icon positions), and the image is
//! then converted to its final compressed form. That needs a macOS session with
//! Finder; when it is not available the image is still produced, without the
//! styled window.
//!
//! Every external tool is built as a plain [`Command`] and handed to the
//! [`Reporter`], so the argument lists and the order of the steps can be
//! asserted without touching the machine.

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};

use super::reset_dir;
use crate::commands::assemble::{Reporter, Step, StepKind};

/// Draws the window background; tinted with the colors of the app's icon.
const BACKGROUND_SCRIPT: &str = include_str!("../../../templates/macos/dmg_background.js");
/// Positions the icons and applies the background through Finder.
const LAYOUT_SCRIPT: &str = include_str!("../../../templates/macos/dmg_layout.applescript");

/// Where the `.dmg` contents are laid out before they are captured.
const STAGING_DIR_NAME: &str = ".dmg-staging";
/// Scratch space of the styling: the scripts and the writable image.
const WORK_DIR_NAME: &str = ".dmg-work";

/// Size of the install window and where its icons sit, in points. Finder
/// positions an icon by its center, measured from the top-left of the window.
const WINDOW_WIDTH: u32 = 660;
const WINDOW_HEIGHT: u32 = 400;
const APP_ICON_X: u32 = 170;
const APPLICATIONS_ICON_X: u32 = 490;
const ICON_Y: u32 = 190;
/// The chevrons sit halfway between the two icons.
const CHEVRON_X: u32 = (APP_ICON_X + APPLICATIONS_ICON_X) / 2;

/// The tool family used to create and convert disk images.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ImageTool {
    /// `diskutil image`, which replaces the deprecated `hdiutil create`.
    Diskutil,
    /// `hdiutil`, for macOS releases that have no `diskutil image`.
    Hdiutil,
}

impl ImageTool {
    /// `diskutil image` where this macOS has it, `hdiutil` otherwise.
    pub(super) fn detect() -> Self {
        let has_image_verb = Command::new("diskutil")
            .args(["image", "--help"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if has_image_verb {
            ImageTool::Diskutil
        } else {
            ImageTool::Hdiutil
        }
    }
}

/// One disk image to build.
pub(super) struct DmgRequest<'a> {
    /// The built (and, if requested, signed) `.app`.
    pub app_path: &'a str,
    pub app_name: &'a str,
    /// Directory holding the staging and scratch folders.
    pub dist: &'a str,
    /// The `.dmg` to create.
    pub output: &'a str,
    pub tool: ImageTool,
    /// Whether a volume with the image's name is already mounted. The styling
    /// asks Finder for the volume by name, so it would lay out that volume.
    pub volume_mounted: bool,
}

/// Whether a volume called `name` is mounted right now.
pub(super) fn volume_is_mounted(name: &str) -> bool {
    Path::new("/Volumes").join(name).exists()
}

/// Build the `.dmg` of `request`.
pub(super) fn build_dmg(request: &DmgRequest, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let staging = format!("{}/{STAGING_DIR_NAME}", request.dist);
    let work = format!("{}/{WORK_DIR_NAME}", request.dist);
    reset_dir(&staging)?;
    reset_dir(&work)?;

    let result = stage_and_create(request, &staging, &work, reporter);
    // The staging copy is a full duplicate of the app; never leave it behind,
    // whether or not the image was created.
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn stage_and_create(
    request: &DmgRequest,
    staging: &str,
    work: &str,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let app_name = request.app_name;
    reporter.note(format!("Staging {app_name}.app for the disk image..."));
    reporter.run(
        ditto_command(request.app_path, &format!("{staging}/{app_name}.app")),
        Step::new(StepKind::Other, "ditto"),
    )?;
    reporter.run(
        applications_link_command(staging),
        Step::new(StepKind::Other, "ln"),
    )?;

    match request.tool {
        ImageTool::Hdiutil => {
            reporter.note(
                "The install window is only styled with `diskutil image`; \
                 creating a plain disk image."
                    .to_string(),
            );
            create_plain_image(request, staging, reporter)
        }
        ImageTool::Diskutil => match create_styled_image(request, staging, work, reporter) {
            Ok(()) => Ok(()),
            Err(error) => {
                reporter.note(format!(
                    "warning: could not style the disk image window ({error:#}); \
                     creating a plain disk image"
                ));
                // Drop what the styling left in the staging folder and any
                // half-written image before starting over.
                let _ = std::fs::remove_dir_all(format!("{staging}/.background"));
                let _ = std::fs::remove_file(request.output);
                create_plain_image(request, staging, reporter)
            }
        },
    }
}

/// A compressed image of the staging folder, without a styled window.
fn create_plain_image(
    request: &DmgRequest,
    staging: &str,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    reporter.note(format!("Creating disk image {}...", request.output));
    match request.tool {
        ImageTool::Diskutil => reporter.run(
            diskutil_create_command("UDZO", Some(request.app_name), staging, request.output),
            Step::new(StepKind::Other, "diskutil image create"),
        ),
        ImageTool::Hdiutil => reporter.run(
            hdiutil_command(request.app_name, staging, request.output),
            Step::new(StepKind::Other, "hdiutil create"),
        ),
    }
}

/// Make the image through a writable copy so Finder can record the window.
///
/// The writable image is always ejected again, whether or not the layout
/// worked, so a failure never leaves a stray volume mounted.
fn create_styled_image(
    request: &DmgRequest,
    staging: &str,
    work: &str,
    reporter: &dyn Reporter,
) -> anyhow::Result<()> {
    let app_name = request.app_name;
    if request.volume_mounted {
        bail!("a volume named '{app_name}' is already mounted; eject it to get the styled window");
    }

    let background_script = format!("{work}/background.js");
    let layout_script = format!("{work}/layout.applescript");
    std::fs::write(&background_script, BACKGROUND_SCRIPT)
        .with_context(|| format!("writing {background_script}"))?;
    std::fs::write(&layout_script, LAYOUT_SCRIPT)
        .with_context(|| format!("writing {layout_script}"))?;
    let background_dir = format!("{staging}/.background");
    std::fs::create_dir_all(&background_dir)
        .with_context(|| format!("creating {background_dir}"))?;
    let app_path = std::path::absolute(request.app_path)
        .with_context(|| format!("resolving {}", request.app_path))?;

    reporter.note("Drawing the install window background...".to_string());
    reporter.run(
        background_command(
            &background_script,
            &format!("{background_dir}/background.png"),
            app_name,
            &app_path.to_string_lossy(),
        ),
        Step::new(StepKind::Other, "disk image background"),
    )?;

    let writable = format!("{work}/writable.sparsebundle");
    reporter.run(
        diskutil_create_command("UDSB", Some(app_name), staging, &writable),
        Step::new(StepKind::Other, "diskutil image create (writable)"),
    )?;
    reporter.run(
        diskutil_attach_command(&writable),
        Step::new(StepKind::Other, "diskutil image attach"),
    )?;

    reporter.note("Laying out the install window in Finder...".to_string());
    let laid_out = reporter.run(
        layout_command(&layout_script, app_name),
        Step::new(StepKind::Other, "disk image layout"),
    );
    let ejected = reporter.run(
        diskutil_eject_command(app_name),
        Step::new(StepKind::Other, "diskutil eject"),
    );
    laid_out?;
    ejected?;

    reporter.note(format!("Creating disk image {}...", request.output));
    reporter.run(
        diskutil_create_command("UDZO", None, &writable, request.output),
        Step::new(StepKind::Other, "diskutil image create"),
    )
}

/// `ditto` keeps symlinks, extended attributes and code signatures intact,
/// which a plain recursive copy does not.
pub(super) fn ditto_command(src: &str, dest: &str) -> Command {
    let mut cmd = Command::new("ditto");
    cmd.arg(src).arg(dest);
    cmd
}

/// The `Applications` shortcut users drag the app onto.
pub(super) fn applications_link_command(staging: &str) -> Command {
    let mut cmd = Command::new("ln");
    cmd.args(["-s", "/Applications"])
        .arg(format!("{staging}/Applications"));
    cmd
}

/// `diskutil image create from`: an image of `source`, a folder or another
/// image, in `format`. The volume name only applies to a folder source.
pub(super) fn diskutil_create_command(
    format: &str,
    volume_name: Option<&str>,
    source: &str,
    output: &str,
) -> Command {
    let mut cmd = Command::new("diskutil");
    cmd.args(["image", "create", "from", "--format", format]);
    if let Some(name) = volume_name {
        cmd.args(["--volumeName", name]);
    }
    cmd.arg(source).arg(output);
    cmd
}

/// Mount a writable image at `/Volumes/<its volume name>`.
pub(super) fn diskutil_attach_command(image: &str) -> Command {
    let mut cmd = Command::new("diskutil");
    cmd.args(["image", "attach"]).arg(image);
    cmd
}

/// Eject the volume Finder just laid out.
pub(super) fn diskutil_eject_command(volume_name: &str) -> Command {
    let mut cmd = Command::new("diskutil");
    cmd.arg("eject").arg(format!("/Volumes/{volume_name}"));
    cmd
}

/// Fallback for macOS releases without `diskutil image`: a compressed (UDZO)
/// read-only image made with the older `hdiutil`.
pub(super) fn hdiutil_command(volume_name: &str, staging: &str, output: &str) -> Command {
    let mut cmd = Command::new("hdiutil");
    cmd.args(["create", "-volname", volume_name, "-srcfolder", staging])
        .args(["-ov", "-format", "UDZO"])
        .arg(output);
    cmd
}

/// Draw the window background into `output`.
pub(super) fn background_command(
    script: &str,
    output: &str,
    app_name: &str,
    app_path: &str,
) -> Command {
    let mut cmd = Command::new("osascript");
    cmd.args(["-l", "JavaScript", script, output, app_name])
        .args([
            WINDOW_WIDTH.to_string(),
            WINDOW_HEIGHT.to_string(),
            CHEVRON_X.to_string(),
            ICON_Y.to_string(),
        ])
        .arg(app_path);
    cmd
}

/// Have Finder lay out the window of the mounted volume.
pub(super) fn layout_command(script: &str, app_name: &str) -> Command {
    let mut cmd = Command::new("osascript");
    cmd.arg(script)
        .arg(app_name)
        .arg(format!("{app_name}.app"))
        .args([
            WINDOW_WIDTH.to_string(),
            WINDOW_HEIGHT.to_string(),
            APP_ICON_X.to_string(),
            APPLICATIONS_ICON_X.to_string(),
            ICON_Y.to_string(),
        ]);
    cmd
}

#[cfg(test)]
mod tests;
