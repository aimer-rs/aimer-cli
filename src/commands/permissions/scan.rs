//! A minimal XML tag scanner.
//!
//! `Info.plist` and `AndroidManifest.xml` are user-owned files, so they are
//! edited as text to keep their formatting and comments. The scanner only has
//! to find tags with their byte offsets: comments, CDATA, processing
//! instructions and the doctype are skipped, so a commented-out entry is never
//! mistaken for a live one.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TagKind {
    Open,
    Close,
    SelfClosing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tag {
    /// Offset of the `<`.
    pub start: usize,
    /// Offset just past the `>`.
    pub end: usize,
    pub name: String,
    pub kind: TagKind,
}

/// Every element tag of `xml`, in document order.
pub(super) fn tags(xml: &str) -> Vec<Tag> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(offset) = xml[at..].find('<') {
        let start = at + offset;
        let rest = &xml[start..];
        if rest.starts_with("<!--") {
            at = skip_past(xml, start, "-->");
        } else if rest.starts_with("<![CDATA[") {
            at = skip_past(xml, start, "]]>");
        } else if rest.starts_with("<?") {
            at = skip_past(xml, start, "?>");
        } else if rest.starts_with("<!") {
            at = skip_past(xml, start, ">");
        } else {
            let Some(end) = tag_end(xml, start) else {
                break;
            };
            found.push(parse(xml, start, end));
            at = end;
        }
    }
    found
}

/// The value of attribute `name` of `tag`, if it has one.
pub(super) fn attribute(xml: &str, tag: &Tag, name: &str) -> Option<String> {
    let text = &xml[tag.start..tag.end];
    let mut from = 0;
    while let Some(offset) = text[from..].find(name) {
        let at = from + offset;
        from = at + name.len();
        let preceded = text[..at].ends_with(char::is_whitespace);
        let after = text[from..].trim_start();
        if !preceded || !after.starts_with('=') {
            continue;
        }
        let value = after[1..].trim_start();
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let inner = &value[1..];
        return Some(inner[..inner.find(quote)?].to_string());
    }
    None
}

fn skip_past(xml: &str, start: usize, terminator: &str) -> usize {
    xml[start..]
        .find(terminator)
        .map_or(xml.len(), |i| start + i + terminator.len())
}

/// The offset past the `>` that closes the tag opening at `start`, ignoring a
/// `>` inside a quoted attribute value.
fn tag_end(xml: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (i, c) in xml[start..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(start + i + 1),
            (None, _) => {}
        }
    }
    None
}

fn parse(xml: &str, start: usize, end: usize) -> Tag {
    let text = &xml[start + 1..end - 1];
    let closing = text.starts_with('/');
    let body = text.trim_start_matches('/');
    let name = body
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default()
        .to_string();
    let kind = if closing {
        TagKind::Close
    } else if text.ends_with('/') {
        TagKind::SelfClosing
    } else {
        TagKind::Open
    };
    Tag {
        start,
        end,
        name,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_found_with_their_kind_and_offsets() {
        let xml = "<a x=\"1\"><b/></a>";
        let found = tags(xml);

        let summary: Vec<_> = found.iter().map(|t| (t.name.as_str(), t.kind)).collect();
        assert_eq!(
            summary,
            [
                ("a", TagKind::Open),
                ("b", TagKind::SelfClosing),
                ("a", TagKind::Close)
            ]
        );
        assert_eq!(&xml[found[0].start..found[0].end], "<a x=\"1\">");
    }

    #[test]
    fn comments_cdata_and_the_prolog_are_skipped() {
        let xml =
            "<?xml version=\"1.0\"?><!DOCTYPE plist><!-- <key>x</key> --><a><![CDATA[<b>]]></a>";

        let names: Vec<_> = tags(xml).into_iter().map(|t| t.name).collect();

        assert_eq!(names, ["a", "a"]);
    }

    #[test]
    fn attributes_are_read_from_either_quote_style() {
        let xml = "<uses-permission android:name=\"a.B\" other='>' />";
        let tag = &tags(xml)[0];

        assert_eq!(attribute(xml, tag, "android:name").as_deref(), Some("a.B"));
        assert_eq!(attribute(xml, tag, "other").as_deref(), Some(">"));
        assert_eq!(attribute(xml, tag, "missing"), None);
        assert_eq!(tag.kind, TagKind::SelfClosing);
    }
}
