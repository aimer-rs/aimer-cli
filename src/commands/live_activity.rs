//! Generating the iOS side of live activities.
//!
//! A project that uses `aimer_live_activity` needs three things the Rust side
//! cannot provide: the Swift bridge Rust looks up with `dlsym`, the
//! `ActivityAttributes` both processes agree on, and a widget extension that
//! draws the activity. `aimer assemble` (and `run` / `package`, which share its
//! iOS staging) reads the project's `Cargo.toml`; when the `live-activity`
//! feature of `aimer` (or `aimer_live_activity` itself) is on, it writes them:
//!
//! - `builds/ios/<package>/AimerLiveActivityBridge.swift` and
//!   `AimerLiveActivityAttributes.swift`, in the app target;
//! - `builds/ios/<package>LiveActivity/`, the extension target's sources and
//!   `Info.plist`;
//! - `NSSupportsLiveActivities` in the app's `Info.plist`;
//! - the extension target in `project.pbxproj`, embedded in the app.
//!
//! Files that exist are never overwritten: the widget is meant to be edited
//! into the app's own layout. Nothing is written for a project that does not
//! use live activities, and a second run changes nothing.

mod detect;
#[cfg(test)]
mod tests;
mod xcodeproj;

use std::fs;
use std::path::Path;

use anyhow::Context;

use crate::commands::assemble::Reporter;
use xcodeproj::{ATTRIBUTES_FILE, BRIDGE_FILE, Extension, PLIST_FILE, WIDGET_FILE};

const BRIDGE_TEMPLATE: &str =
    include_str!("../../templates/ios/live_activity/AimerLiveActivityBridge.swift.template");
const ATTRIBUTES_TEMPLATE: &str =
    include_str!("../../templates/ios/live_activity/AimerLiveActivityAttributes.swift.template");
const WIDGET_TEMPLATE: &str =
    include_str!("../../templates/ios/live_activity/AimerLiveActivityWidget.swift.template");
const EXTENSION_PLIST_TEMPLATE: &str =
    include_str!("../../templates/ios/live_activity/Info.plist.template");

/// The key that lets the app start live activities.
const SUPPORTS_KEY: &str = "NSSupportsLiveActivities";

/// Suffix of the extension's name and bundle identifier.
const EXTENSION_SUFFIX: &str = "LiveActivity";

/// Bring the iOS project of `pkg_name` up to what live activities need, if the
/// project uses them. Run from the project root.
pub(crate) fn sync_ios(pkg_name: &str, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let Ok(manifest) = fs::read_to_string("Cargo.toml") else {
        return Ok(());
    };
    if !detect::enabled(&manifest) {
        return Ok(());
    }
    sync_project(Path::new(crate::commands::assemble::IOS_PROJECT_DIR), pkg_name, reporter)
}

/// [`sync_ios`] for the iOS project in `ios_dir`, once the project is known to
/// use live activities.
fn sync_project(ios_dir: &Path, pkg_name: &str, reporter: &dyn Reporter) -> anyhow::Result<()> {
    let app_dir = ios_dir.join(pkg_name);
    let plist_path = app_dir.join(PLIST_FILE);
    let Ok(plist) = fs::read_to_string(&plist_path) else {
        reporter.note(format!(
            "warning: Live activity: {} not found; run `aimer migrate ios` to create the iOS project",
            plist_path.display()
        ));
        return Ok(());
    };
    let project_path = ios_dir.join(format!("{pkg_name}.xcodeproj/project.pbxproj"));
    let project = fs::read_to_string(&project_path).ok();
    let Some(app_id) = bundle_identifier(&plist)
        .or_else(|| project.as_deref().and_then(project_bundle_identifier))
    else {
        reporter.note(format!(
            "warning: Live activity: no bundle identifier in {} or in the Xcode project; add the widget extension in Xcode by hand",
            plist_path.display()
        ));
        return Ok(());
    };

    let name = format!("{pkg_name}{EXTENSION_SUFFIX}");
    let module = identifier(&name);
    let bundle_id = format!("{app_id}.{EXTENSION_SUFFIX}");
    let extension_dir = ios_dir.join(&name);
    fs::create_dir_all(&extension_dir)
        .with_context(|| format!("creating {}", extension_dir.display()))?;

    let mut written = Vec::new();
    for (path, contents) in [
        (app_dir.join(BRIDGE_FILE), BRIDGE_TEMPLATE.to_owned()),
        (app_dir.join(ATTRIBUTES_FILE), ATTRIBUTES_TEMPLATE.to_owned()),
        (extension_dir.join(WIDGET_FILE), WIDGET_TEMPLATE.to_owned()),
        (
            extension_dir.join(PLIST_FILE),
            EXTENSION_PLIST_TEMPLATE.replace("${display_name}", &name),
        ),
    ] {
        if write_missing(&path, &contents)? {
            written.push(path.display().to_string());
        }
    }

    let with_key = with_supports_key(&plist);
    if with_key != plist {
        fs::write(&plist_path, with_key)
            .with_context(|| format!("writing {}", plist_path.display()))?;
        written.push(format!("{SUPPORTS_KEY} in {}", plist_path.display()));
    }

    let mut manual = None;
    match project {
        None => manual = Some(format!("{} not found", project_path.display())),
        Some(project) if xcodeproj::has_extension(&project) => {
            // Projects generated before the bundle name was pinned.
            if let Some(patched) = xcodeproj::pin_wrapper_name(&project, &name) {
                fs::write(&project_path, patched)
                    .with_context(|| format!("writing {}", project_path.display()))?;
                written.push(format!("bundle name of {name}.appex pinned in {}", project_path.display()));
            }
        }
        Some(project) => {
            let extension = Extension { name: &name, module: &module, bundle_id: &bundle_id };
            match xcodeproj::add_extension(&project, &extension) {
                Ok(patched) => {
                    fs::write(&project_path, patched)
                        .with_context(|| format!("writing {}", project_path.display()))?;
                    written.push(format!("target {name} in {}", project_path.display()));
                }
                Err(why) => manual = Some(why),
            }
        }
    }

    announce(reporter, &written, manual.as_deref(), &name, &bundle_id);
    Ok(())
}

/// Tell the user what was generated and what is still theirs to do. Silent
/// when there is nothing new, so a build does not repeat itself.
fn announce(
    reporter: &dyn Reporter,
    written: &[String],
    manual: Option<&str>,
    name: &str,
    bundle_id: &str,
) {
    if !written.is_empty() {
        reporter.note("Live activity: `live-activity` is on, so the iOS project was updated:".to_owned());
        for item in written {
            reporter.note(format!("  + {item}"));
        }
        reporter.note(format!(
            "Live activity: the widget extension `{name}` has the bundle identifier `{bundle_id}`; \
             register that App ID, and give the extension a signing profile, before shipping."
        ));
        reporter.note(format!(
            "Live activity: edit builds/ios/{name}/{WIDGET_FILE} to change how the activity looks; \
             it is never overwritten."
        ));
    }
    if let Some(why) = manual {
        reporter.note(format!(
            "warning: Live activity: the Xcode project was not changed ({why}). Add a Widget Extension \
             target named `{name}` with {WIDGET_FILE} and {ATTRIBUTES_FILE}, embed it in the app, and add \
             {BRIDGE_FILE} and {ATTRIBUTES_FILE} to the app target."
        ));
    }
}

/// Writes `contents` to `path` unless a file is there. Whether it wrote.
fn write_missing(path: &Path, contents: &str) -> anyhow::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// The `CFBundleIdentifier` of an `Info.plist`, when it is spelled out rather
/// than a build setting.
fn bundle_identifier(plist: &str) -> Option<String> {
    let after = plist.split_once("<key>CFBundleIdentifier</key>")?.1;
    let value = after.split_once("<string>")?.1.split_once("</string>")?.0.trim();
    (!value.is_empty() && !value.contains("$(")).then(|| value.to_owned())
}

/// The `PRODUCT_BUNDLE_IDENTIFIER` an Xcode project sets for the app, for an
/// `Info.plist` that refers to the build setting. The extension's own setting
/// is skipped, so the answer does not change once it has been added.
fn project_bundle_identifier(pbxproj: &str) -> Option<String> {
    const SETTING: &str = "PRODUCT_BUNDLE_IDENTIFIER = ";
    pbxproj
        .match_indices(SETTING)
        .filter_map(|(at, _)| {
            let rest = &pbxproj[at + SETTING.len()..];
            let value = rest.split_once(';')?.0.trim().trim_matches('"');
            (!value.is_empty() && !value.contains("$(")).then(|| value.to_owned())
        })
        .find(|value| !value.ends_with(&format!(".{EXTENSION_SUFFIX}")))
}

/// `plist` with [`SUPPORTS_KEY`] set, left as it is when already there.
fn with_supports_key(plist: &str) -> String {
    if plist.contains(SUPPORTS_KEY) {
        return plist.to_owned();
    }
    // The last `</dict>` closes the top-level dictionary.
    let Some(at) = plist.rfind("</dict>") else {
        return plist.to_owned();
    };
    let mut out = plist.to_owned();
    out.insert_str(at, &format!("    <key>{SUPPORTS_KEY}</key>\n    <true/>\n"));
    out
}

/// `name` as a Swift module name: letters, digits and `_`, not starting with a
/// digit.
fn identifier(name: &str) -> String {
    let mut out: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}
