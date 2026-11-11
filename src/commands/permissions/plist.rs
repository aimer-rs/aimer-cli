//! Syncing `[permissions]` into an `Info.plist`.

use anyhow::{Context, bail};

use super::block;
use super::scan::{self, Tag, TagKind};

/// A hand-written entry that `Aimer.toml` replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Overridden {
    pub key: String,
    /// The value the file had.
    pub previous: String,
    /// The value `Aimer.toml` sets.
    pub new: String,
}

/// The outcome of [`sync`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlistSync {
    pub text: String,
    /// Keys written into the generated block.
    pub added: Vec<String>,
    /// Hand-written entries commented out because `Aimer.toml` sets another value.
    pub overridden: Vec<Overridden>,
    /// Keys the file already sets to the same value, so nothing was written.
    pub kept: Vec<String>,
    /// Whether a block from an earlier sync was removed.
    pub removed_block: bool,
}

/// A `<key>` with its value, as found at the top level of the plist.
struct Entry {
    key: String,
    start: usize,
    end: usize,
    /// The text of a `<string>` value; `None` for any other type.
    string: Option<String>,
}

/// Make `text` hold `entries` (key, string value) in a generated block.
///
/// The block of an earlier sync is rebuilt from scratch. A key the file sets
/// itself with the same value is left alone; with another value, the file's
/// entry is commented out in place (never deleted) and `Aimer.toml` wins.
pub(super) fn sync(text: &str, entries: &[(String, String)]) -> anyhow::Result<PlistSync> {
    let (stripped, removed_block) = block::strip(text)?;
    let mut sync = PlistSync {
        text: stripped.clone(),
        added: Vec::new(),
        overridden: Vec::new(),
        kept: Vec::new(),
        removed_block,
    };
    if entries.is_empty() {
        return Ok(sync);
    }

    let (existing, dict_close) = top_level(&stripped)?;
    let mut lines = Vec::new();
    let mut to_comment: Vec<&Entry> = Vec::new();
    for (key, value) in entries {
        match existing.iter().find(|entry| &entry.key == key) {
            Some(entry) if entry.string.as_deref() == Some(value) => sync.kept.push(key.clone()),
            found => {
                if let Some(entry) = found {
                    to_comment.push(entry);
                    sync.overridden.push(Overridden {
                        key: key.clone(),
                        previous: entry
                            .string
                            .clone()
                            .unwrap_or_else(|| stripped[entry.start..entry.end].to_string()),
                        new: value.clone(),
                    });
                }
                lines.push(format!("<key>{}</key>", escape(key)));
                lines.push(format!("<string>{}</string>", escape(value)));
                sync.added.push(key.clone());
            }
        }
    }

    let mut text = stripped;
    if !lines.is_empty() {
        insert_block(&mut text, dict_close, &lines);
    }
    // The entries all precede the closing tag, so commenting them out from the
    // last one keeps the earlier offsets valid.
    to_comment.sort_by_key(|entry| std::cmp::Reverse(entry.start));
    for entry in to_comment {
        let original = text[entry.start..entry.end].replace("--", "- -");
        text.replace_range(
            entry.start..entry.end,
            &format!("<!-- aimer:overridden by [permissions] in Aimer.toml: {original} -->"),
        );
    }
    sync.text = text;
    Ok(sync)
}

/// Put the generated block before the `</dict>` at `dict_close`.
fn insert_block(text: &mut String, dict_close: usize, lines: &[String]) {
    let line_start = text[..dict_close].rfind('\n').map_or(0, |i| i + 1);
    let leading = &text[line_start..dict_close];
    if leading.trim().is_empty() {
        let indent = format!("{leading}    ");
        text.insert_str(line_start, &block::render(&indent, lines));
    } else {
        let block = format!("\n{}", block::render("    ", lines));
        text.insert_str(dict_close, &block);
    }
}

/// The entries of the top-level `<dict>` and the offset of its closing tag.
fn top_level(text: &str) -> anyhow::Result<(Vec<Entry>, usize)> {
    let tags = scan::tags(text);
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < tags.len() {
        let tag = &tags[i];
        match tag.kind {
            TagKind::Open if tag.name == "key" && depth == 2 => {
                let close = tags
                    .get(i + 1)
                    .filter(|t| t.kind == TagKind::Close && t.name == "key")
                    .context("an Info.plist <key> is not closed")?;
                let key = unescape(&text[tag.end..close.start]);
                let value = tags
                    .get(i + 2)
                    .context("an Info.plist <key> has no value")?;
                let (end, next) = value_end(&tags, i + 2)?;
                let string = match tags.get(i + 3) {
                    Some(c) if value.name == "string" && value.kind == TagKind::Open => {
                        Some(unescape(&text[value.end..c.start]))
                    }
                    _ if value.name == "string" && value.kind == TagKind::SelfClosing => {
                        Some(String::new())
                    }
                    _ => None,
                };
                entries.push(Entry {
                    key,
                    start: tag.start,
                    end,
                    string,
                });
                i = next;
                continue;
            }
            TagKind::Open => depth += 1,
            TagKind::Close => {
                depth = depth.saturating_sub(1);
                if depth == 1 && tag.name == "dict" {
                    return Ok((entries, tag.start));
                }
            }
            TagKind::SelfClosing => {}
        }
        i += 1;
    }
    bail!("the Info.plist has no top-level <dict>")
}

/// The offset past the value starting at tag `at`, and the index of the tag
/// after it.
fn value_end(tags: &[Tag], at: usize) -> anyhow::Result<(usize, usize)> {
    match tags[at].kind {
        TagKind::SelfClosing => Ok((tags[at].end, at + 1)),
        TagKind::Open => {
            let mut depth = 0usize;
            for (k, tag) in tags.iter().enumerate().skip(at) {
                match tag.kind {
                    TagKind::Open => depth += 1,
                    TagKind::Close => depth -= 1,
                    TagKind::SelfClosing => {}
                }
                if depth == 0 {
                    return Ok((tag.end, k + 1));
                }
            }
            bail!("an Info.plist value is not closed")
        }
        TagKind::Close => bail!("an Info.plist <key> has no value"),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape(text: &str) -> String {
    text.trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n    <key>CFBundleName</key>\n    <string>App</string>\n    <key>UIFlags</key>\n    <dict>\n        <key>NSCameraUsageDescription</key>\n        <string>nested, not ours</string>\n    </dict>\n</dict>\n</plist>\n";

    fn entry(key: &str, value: &str) -> (String, String) {
        (key.to_string(), value.to_string())
    }

    #[test]
    fn entries_go_in_a_marked_block_before_the_closing_dict() {
        let out = sync(PLIST, &[entry("NSCameraUsageDescription", "Scan <codes>")]).unwrap();

        assert_eq!(out.added, ["NSCameraUsageDescription"]);
        assert!(out.overridden.is_empty());
        assert!(out.text.contains(
            "    <!-- aimer:permissions:start: generated from [permissions] in Aimer.toml, edits between the markers are overwritten -->\n    <key>NSCameraUsageDescription</key>\n    <string>Scan &lt;codes&gt;</string>\n    <!-- aimer:permissions:end -->\n</dict>\n</plist>"
        ));
        assert!(out.text.contains("nested, not ours"));
    }

    #[test]
    fn a_second_sync_changes_nothing() {
        let wanted = [entry("NSCameraUsageDescription", "Scan")];
        let first = sync(PLIST, &wanted).unwrap();

        let second = sync(&first.text, &wanted).unwrap();

        assert_eq!(second.text, first.text);
        assert!(second.removed_block);
    }

    #[test]
    fn a_different_hand_written_value_is_commented_out_and_reported() {
        let plist = PLIST.replace(
            "<key>CFBundleName</key>",
            "<key>NSCameraUsageDescription</key>\n    <string>Mine</string>\n    <key>CFBundleName</key>",
        );

        let out = sync(&plist, &[entry("NSCameraUsageDescription", "Theirs")]).unwrap();

        assert_eq!(
            out.overridden,
            [Overridden {
                key: "NSCameraUsageDescription".into(),
                previous: "Mine".into(),
                new: "Theirs".into()
            }]
        );
        assert!(out.text.contains(
            "<!-- aimer:overridden by [permissions] in Aimer.toml: <key>NSCameraUsageDescription</key>\n    <string>Mine</string> -->"
        ));
        assert!(out.text.contains("<string>Theirs</string>"));
        // The live entries: the nested one plus ours; the user's is a comment now.
        let live_keys = scan::tags(&out.text)
            .iter()
            .filter(|t| t.name == "key" && t.kind == TagKind::Open)
            .count();
        assert_eq!(live_keys, 4);
        let again = sync(&out.text, &[entry("NSCameraUsageDescription", "Theirs")]).unwrap();
        assert_eq!(again.text, out.text);
    }

    #[test]
    fn the_same_hand_written_value_is_left_alone() {
        let plist = PLIST.replace(
            "<key>CFBundleName</key>",
            "<key>NSCameraUsageDescription</key>\n    <string>Same</string>\n    <key>CFBundleName</key>",
        );

        let out = sync(&plist, &[entry("NSCameraUsageDescription", "Same")]).unwrap();

        assert_eq!(out.kept, ["NSCameraUsageDescription"]);
        assert_eq!(out.text, plist);
    }

    #[test]
    fn dropping_every_permission_removes_the_block() {
        let with = sync(PLIST, &[entry("NSCameraUsageDescription", "Scan")]).unwrap();

        let without = sync(&with.text, &[]).unwrap();

        assert_eq!(without.text, PLIST);
        assert!(without.removed_block);
    }

    #[test]
    fn a_plist_without_a_top_level_dict_is_an_error() {
        assert!(sync("<plist></plist>", &[entry("A", "b")]).is_err());
    }
}
