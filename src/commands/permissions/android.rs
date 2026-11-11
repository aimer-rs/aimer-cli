//! Syncing `[permissions]` into an `AndroidManifest.xml`.

use anyhow::Context;

use super::block;
use super::scan::{self, TagKind};
use crate::config::AndroidUse;

/// The outcome of [`sync`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AndroidSync {
    pub text: String,
    /// Permissions written into the generated block.
    pub added: Vec<String>,
    /// Permissions the manifest already declares itself, so nothing was written.
    pub kept: Vec<String>,
    /// Whether a block from an earlier sync was removed.
    pub removed_block: bool,
}

/// Make `text` declare `uses` in a generated block right after `<manifest>`.
///
/// The block of an earlier sync is rebuilt from scratch. A permission the
/// manifest declares itself is never duplicated: the user's element stays.
pub(super) fn sync(text: &str, uses: &[AndroidUse]) -> anyhow::Result<AndroidSync> {
    let (stripped, removed_block) = block::strip(text)?;
    let mut sync = AndroidSync {
        text: stripped.clone(),
        added: Vec::new(),
        kept: Vec::new(),
        removed_block,
    };
    if uses.is_empty() {
        return Ok(sync);
    }

    let tags = scan::tags(&stripped);
    let declared: Vec<String> = tags
        .iter()
        .filter(|tag| tag.name == "uses-permission" && tag.kind != TagKind::Close)
        .filter_map(|tag| scan::attribute(&stripped, tag, "android:name"))
        .collect();
    let mut lines = Vec::new();
    for permission in uses {
        if declared.contains(&permission.name) {
            sync.kept.push(permission.name.clone());
            continue;
        }
        let max_sdk = permission
            .max_sdk_version
            .map(|level| format!(" android:maxSdkVersion=\"{level}\""))
            .unwrap_or_default();
        lines.push(format!(
            "<uses-permission android:name=\"{}\"{max_sdk} />",
            permission.name
        ));
        sync.added.push(permission.name.clone());
    }
    if lines.is_empty() {
        return Ok(sync);
    }

    let manifest = tags
        .iter()
        .find(|tag| tag.name == "manifest" && tag.kind == TagKind::Open)
        .context("the AndroidManifest.xml has no <manifest> element")?;
    let block = format!("\n{}", block::render("    ", &lines));
    let mut text = stripped;
    // The block ends in a newline of its own; the one the manifest tag was
    // followed by now trails it.
    text.insert_str(manifest.end, block.trim_end_matches('\n'));
    sync.text = text;
    Ok(sync)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n\n    <application android:label=\"App\" />\n\n</manifest>\n";

    fn uses(names: &[&str]) -> Vec<AndroidUse> {
        names
            .iter()
            .map(|name| AndroidUse {
                name: name.to_string(),
                max_sdk_version: None,
            })
            .collect()
    }

    #[test]
    fn permissions_go_in_a_marked_block_after_the_manifest_tag() {
        let out = sync(MANIFEST, &uses(&["android.permission.CAMERA"])).unwrap();

        assert_eq!(out.added, ["android.permission.CAMERA"]);
        assert!(out.text.contains(
            ">\n    <!-- aimer:permissions:start: generated from [permissions] in Aimer.toml, edits between the markers are overwritten -->\n    <uses-permission android:name=\"android.permission.CAMERA\" />\n    <!-- aimer:permissions:end -->\n\n    <application"
        ));
    }

    #[test]
    fn a_maximum_sdk_version_is_written() {
        let wanted = [AndroidUse {
            name: "android.permission.READ_EXTERNAL_STORAGE".into(),
            max_sdk_version: Some(32),
        }];

        let out = sync(MANIFEST, &wanted).unwrap();

        assert!(out.text.contains(
            "<uses-permission android:name=\"android.permission.READ_EXTERNAL_STORAGE\" android:maxSdkVersion=\"32\" />"
        ));
    }

    #[test]
    fn a_second_sync_changes_nothing_and_dropping_everything_restores_the_file() {
        let wanted = uses(&["android.permission.CAMERA"]);
        let first = sync(MANIFEST, &wanted).unwrap();

        let second = sync(&first.text, &wanted).unwrap();
        let dropped = sync(&first.text, &[]).unwrap();

        assert_eq!(second.text, first.text);
        assert_eq!(dropped.text, MANIFEST);
        assert!(dropped.removed_block);
    }

    #[test]
    fn a_permission_the_user_declared_is_not_duplicated() {
        let manifest = MANIFEST.replace(
            "    <application",
            "    <uses-permission android:name=\"android.permission.CAMERA\"/>\n    <!-- <uses-permission android:name=\"android.permission.VIBRATE\"/> -->\n    <application",
        );

        let out = sync(
            &manifest,
            &uses(&["android.permission.CAMERA", "android.permission.VIBRATE"]),
        )
        .unwrap();

        assert_eq!(out.kept, ["android.permission.CAMERA"]);
        // A commented-out declaration is not a declaration.
        assert_eq!(out.added, ["android.permission.VIBRATE"]);
        assert_eq!(out.text.matches("android.permission.CAMERA").count(), 1);
    }

    #[test]
    fn nothing_to_add_leaves_the_file_untouched() {
        let manifest = MANIFEST.replace(
            "    <application",
            "    <uses-permission android:name=\"android.permission.CAMERA\"/>\n    <application",
        );

        let out = sync(&manifest, &uses(&["android.permission.CAMERA"])).unwrap();

        assert_eq!(out.text, manifest);
    }

    #[test]
    fn a_manifest_without_a_manifest_element_is_an_error() {
        assert!(sync("<application/>", &uses(&["a.B"])).is_err());
    }
}
