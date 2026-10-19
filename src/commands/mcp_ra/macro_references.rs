use super::source_edits::PreviewFile;
use super::*;

#[derive(Clone, Debug)]
pub(super) struct MacroReferenceReport {
    pub(super) symbol: Option<String>,
    pub(super) candidates: Vec<MacroReferenceCandidate>,
    pub(super) scanned_files: usize,
    pub(super) scan_complete: bool,
    pub(super) truncated: bool,
    pub(super) issue_count: usize,
}

#[derive(Clone, Debug)]
pub(super) struct MacroReferenceCandidate {
    file: String,
    line: usize,
    column: usize,
    macro_name: String,
    text: String,
}

pub(super) fn diagnostic_severity_name(severity: u64) -> &'static str {
    match severity {
        1 => "error",
        2 => "warning",
        3 => "information",
        4 => "hint",
        _ => "unknown",
    }
}

pub(super) fn position_from_source(source: &str, line: u32, column: u32) -> anyhow::Result<Value> {
    if line == 0 || column == 0 {
        bail!("line and column are 1-based and must be greater than zero");
    }
    let line_text = source
        .lines()
        .nth((line - 1) as usize)
        .ok_or_else(|| anyhow!("line is outside the source file"))?;
    let requested_units = (column - 1) as usize;
    let mut units = 0;
    for ch in line_text.chars() {
        if units == requested_units {
            break;
        }
        units += ch.len_utf16();
        if units > requested_units {
            bail!("column splits a UTF-16 surrogate pair");
        }
    }
    if units != requested_units {
        bail!("column is outside the source line");
    }
    Ok(json!({ "line": line - 1, "character": requested_units }))
}

pub(super) fn source_end_position(source: &str) -> Value {
    let last_line = source.rsplit('\n').next().unwrap_or_default();
    let last_line = last_line.strip_suffix('\r').unwrap_or(last_line);
    json!({
        "line": source.bytes().filter(|byte| *byte == b'\n').count(),
        "character": last_line.encode_utf16().count()
    })
}

pub(super) fn source_offset(source: &str, position: &Value) -> anyhow::Result<usize> {
    let line = position.get("line").and_then(Value::as_u64).unwrap_or(0) as usize;
    let character = position
        .get("character")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut offset = 0;
    for (index, line_text) in source.split_inclusive('\n').enumerate() {
        if index == line {
            let mut units = 0;
            for (byte, ch) in line_text.char_indices() {
                if units >= character {
                    return Ok(offset + byte);
                }
                units += ch.len_utf16();
            }
            let content = line_text.trim_end_matches(['\r', '\n']);
            let content_units = content.encode_utf16().count();
            if character == content_units {
                return Ok(offset + content.len());
            }
            bail!("LSP edit position is outside the source line");
        }
        offset += line_text.len();
    }
    if line == source.split('\n').count().saturating_sub(1) && character == 0 {
        return Ok(source.len());
    }
    bail!("LSP edit line is outside the source file")
}

pub(super) fn identifier_at_lsp_position(source: &str, position: &Value) -> anyhow::Result<String> {
    let offset = source_offset(source, position)?;
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let column = offset - line_start;
    let tokens = source
        .parse::<proc_macro2::TokenStream>()
        .map_err(|error| anyhow::anyhow!("tokenizing Rust source at rename position: {error}"))?;
    identifier_at_token_position(tokens, line, column)
        .ok_or_else(|| anyhow!("could not identify the Rust symbol at the rename position"))
}

pub(super) fn identifier_at_token_position(
    tokens: proc_macro2::TokenStream,
    line: usize,
    column: usize,
) -> Option<String> {
    tokens.into_iter().find_map(|token| match token {
        proc_macro2::TokenTree::Ident(identifier) => {
            let span = identifier.span();
            let start = span.start();
            let end = span.end();
            (start.line == line
                && end.line == line
                && start.column <= column
                && column <= end.column)
                .then(|| normalized_ident_name(&identifier.to_string()))
        }
        proc_macro2::TokenTree::Group(group) => {
            identifier_at_token_position(group.stream(), line, column)
        }
        _ => None,
    })
}

pub(super) fn normalized_ident_name(identifier: &str) -> String {
    identifier
        .strip_prefix("r#")
        .unwrap_or(identifier)
        .to_owned()
}

pub(super) async fn find_unedited_macro_references(
    root: &Path,
    preview_files: &[PreviewFile],
    old_name: Option<&str>,
) -> MacroReferenceReport {
    let mut report = MacroReferenceReport {
        symbol: old_name.map(str::to_owned),
        candidates: Vec::new(),
        scanned_files: 0,
        scan_complete: old_name.is_some(),
        truncated: false,
        issue_count: 0,
    };
    let Some(old_name) = old_name else {
        report.issue_count = 1;
        return report;
    };

    let root = match tokio::fs::canonicalize(root).await {
        Ok(root) => root,
        Err(_) => {
            report.scan_complete = false;
            report.issue_count = 1;
            return report;
        }
    };
    let edited_sources = preview_files
        .iter()
        .filter_map(|file| {
            file.after
                .as_deref()
                .map(|after| (file.path.clone(), after))
        })
        .collect::<HashMap<_, _>>();
    let mut directories = vec![root.clone()];
    let mut source_files = Vec::new();

    while let Some(directory) = directories.pop() {
        let mut entries = match tokio::fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(_) => {
                report.scan_complete = false;
                report.issue_count += 1;
                continue;
            }
        };
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    break;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            };
            if file_type.is_dir() {
                let name = entry.file_name();
                if matches!(
                    name.to_str(),
                    Some(".git" | ".codegraph" | ".aimer" | "target")
                ) {
                    continue;
                }
                directories.push(path);
            } else if file_type.is_file()
                && path.extension().and_then(std::ffi::OsStr::to_str) == Some("rs")
            {
                source_files.push(path);
            }
        }
    }
    source_files.sort();

    for path in source_files {
        let source = if let Some(source) = edited_sources.get(&path) {
            match std::str::from_utf8(source) {
                Ok(source) => source.to_owned(),
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            }
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(source) => source,
                Err(_) => {
                    report.scan_complete = false;
                    report.issue_count += 1;
                    continue;
                }
            }
        };
        let syntax = match syn::parse_file(&source) {
            Ok(syntax) => syntax,
            Err(_) => {
                report.scan_complete = false;
                report.issue_count += 1;
                continue;
            }
        };
        report.scanned_files += 1;
        let relative = path
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().into_owned());
        let lines = source.lines().collect::<Vec<_>>();
        let mut visitor = MacroReferenceVisitor {
            old_name,
            file: &relative,
            lines: &lines,
            candidates: &mut report.candidates,
            truncated: &mut report.truncated,
        };
        visitor.visit_file(&syntax);
    }

    report
}

struct MacroReferenceVisitor<'a> {
    old_name: &'a str,
    file: &'a str,
    lines: &'a [&'a str],
    candidates: &'a mut Vec<MacroReferenceCandidate>,
    truncated: &'a mut bool,
}

impl<'ast> Visit<'ast> for MacroReferenceVisitor<'_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item.mac.path.is_ident("macro_rules") {
            let macro_name = item
                .ident
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "<unnamed>".to_owned());
            collect_macro_identifiers(
                item.mac.tokens.clone(),
                self.old_name,
                self.file,
                &macro_name,
                self.lines,
                self.candidates,
                self.truncated,
            );
        }
        syn::visit::visit_item_macro(self, item);
    }
}

pub(super) fn collect_macro_identifiers(
    tokens: proc_macro2::TokenStream,
    old_name: &str,
    file: &str,
    macro_name: &str,
    lines: &[&str],
    candidates: &mut Vec<MacroReferenceCandidate>,
    truncated: &mut bool,
) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Ident(identifier)
                if normalized_ident_name(&identifier.to_string()) == old_name =>
            {
                if candidates.len() >= MAX_RESULTS {
                    *truncated = true;
                    continue;
                }
                let start = identifier.span().start();
                let Some(line_text) = start.line.checked_sub(1).and_then(|line| lines.get(line))
                else {
                    *truncated = true;
                    continue;
                };
                let byte_column = start.column.min(line_text.len());
                let column = line_text[..byte_column].encode_utf16().count() + 1;
                candidates.push(MacroReferenceCandidate {
                    file: file.to_owned(),
                    line: start.line,
                    column,
                    macro_name: macro_name.to_owned(),
                    text: line_text.trim().to_owned(),
                });
            }
            proc_macro2::TokenTree::Group(group) => collect_macro_identifiers(
                group.stream(),
                old_name,
                file,
                macro_name,
                lines,
                candidates,
                truncated,
            ),
            _ => {}
        }
    }
}

pub(super) fn macro_reference_report_json(report: &MacroReferenceReport) -> Value {
    json!({
        "symbol": report.symbol,
        "candidate_edits_added": false,
        "scan_complete": report.scan_complete,
        "scanned_files": report.scanned_files,
        "issue_count": report.issue_count,
        "truncated": report.truncated,
        "review_note": "These are possible old-name tokens remaining in macro_rules! bodies. They are unverified and were not added to the rename edit.",
        "items": report.candidates.iter().map(|candidate| json!({
            "file": candidate.file,
            "line": candidate.line,
            "column": candidate.column,
            "macro_name": candidate.macro_name,
            "identifier": report.symbol,
            "verified_reference": false,
            "text": candidate.text
        })).collect::<Vec<_>>()
    })
}
