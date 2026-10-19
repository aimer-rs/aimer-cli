use super::*;

pub async fn inspect_file(
    root: &Path,
    file: &str,
    ranges: &[InspectFileRange],
    searches: &[InspectFilePattern],
) -> anyhow::Result<Value> {
    if file.trim().is_empty() {
        bail!("file must not be empty");
    }
    if ranges.len() > MAX_INSPECT_RANGES {
        bail!("ranges must contain at most {MAX_INSPECT_RANGES} items");
    }
    if searches.len() > MAX_INSPECT_SEARCHES {
        bail!("searches must contain at most {MAX_INSPECT_SEARCHES} items");
    }
    if ranges.is_empty() && searches.is_empty() {
        bail!("provide at least one line range or search pattern");
    }

    let mut requested_range_lines = 0usize;
    for range in ranges {
        if range.start_line == 0 || range.end_line < range.start_line {
            bail!(
                "invalid line range {}..={}",
                range.start_line,
                range.end_line
            );
        }
        let range_line_count = range
            .end_line
            .checked_sub(range.start_line)
            .and_then(|count| count.checked_add(1))
            .context("line range is too large")? as usize;
        requested_range_lines = requested_range_lines
            .checked_add(range_line_count)
            .context("requested line ranges are too large")?;
        if requested_range_lines > MAX_INSPECT_RANGE_LINES {
            bail!("requested ranges may include at most {MAX_INSPECT_RANGE_LINES} lines");
        }
    }

    let mut compiled_searches = Vec::with_capacity(searches.len());
    for search in searches {
        if search.pattern.trim().is_empty() {
            bail!("search pattern must not be empty");
        }
        if search.pattern.len() > 512 {
            bail!("search pattern must be at most 512 bytes");
        }
        if search.context_before > MAX_INSPECT_CONTEXT_LINES
            || search.context_after > MAX_INSPECT_CONTEXT_LINES
        {
            bail!("search context must be between 0 and {MAX_INSPECT_CONTEXT_LINES} lines");
        }
        let expression = if search.regex {
            search.pattern.clone()
        } else {
            regex::escape(&search.pattern)
        };
        let compiled = RegexBuilder::new(&expression)
            .case_insensitive(!search.case_sensitive)
            .build()
            .with_context(|| format!("invalid regex pattern '{}'", search.pattern))?;
        compiled_searches.push(compiled);
    }

    let workspace_root = root
        .canonicalize()
        .with_context(|| format!("resolving selected Rust workspace '{}'", root.display()))?;
    let requested_file = Path::new(file);
    if requested_file.is_absolute() {
        bail!("file must be project-relative");
    }
    let resolved_file = workspace_root
        .join(requested_file)
        .canonicalize()
        .with_context(|| format!("resolving project file '{file}'"))?;
    if !resolved_file.starts_with(&workspace_root) {
        bail!("file resolves outside the selected Rust project");
    }
    let metadata = tokio::fs::metadata(&resolved_file)
        .await
        .with_context(|| format!("reading file metadata for '{file}'"))?;
    if !metadata.is_file() {
        bail!("'{file}' is not a regular file");
    }
    if metadata.len() > MAX_INSPECT_FILE_BYTES {
        bail!("file is larger than the {MAX_INSPECT_FILE_BYTES}-byte inspection limit");
    }
    let contents = tokio::fs::read_to_string(&resolved_file)
        .await
        .with_context(|| format!("reading UTF-8 text from '{file}'"))?;
    let lines = contents.lines().collect::<Vec<_>>();

    let mut range_results = Vec::with_capacity(ranges.len());
    let mut any_line_truncated = false;
    let mut truncated_lines = BTreeSet::new();
    for range in ranges {
        let start_index = (range.start_line as usize - 1).min(lines.len());
        let end_index = (range.end_line as usize).min(lines.len()).max(start_index);
        let mut range_lines = Vec::with_capacity(end_index.saturating_sub(start_index));
        for line_index in start_index..end_index {
            let (line, truncated) = bounded_inspect_text(lines[line_index]);
            if truncated {
                any_line_truncated = true;
                truncated_lines.insert(line_index + 1);
            }
            range_lines.push(line);
        }
        range_results.push(json!({
            "start_line": range.start_line,
            "text": range_lines.join("\n")
        }));
    }

    let mut total_matches = 0usize;
    let mut any_search_truncated = false;
    let mut search_results = Vec::with_capacity(searches.len());
    for (search, expression) in searches.iter().zip(compiled_searches.iter()) {
        let mut matches = Vec::new();
        let mut search_truncated = false;
        for (line_index, line) in lines.iter().enumerate() {
            let Some(found) = expression.find(line) else {
                continue;
            };
            if total_matches >= MAX_INSPECT_MATCHES {
                search_truncated = true;
                any_search_truncated = true;
                break;
            }
            total_matches += 1;
            let (match_text, text_truncated) = bounded_inspect_text(line);
            if text_truncated {
                any_line_truncated = true;
                truncated_lines.insert(line_index + 1);
            }
            let column = line[..found.start()].chars().count() + 1;
            let before_start = line_index.saturating_sub(search.context_before);
            let after_end = line_index
                .saturating_add(1)
                .saturating_add(search.context_after)
                .min(lines.len());
            let context_before = (before_start..line_index)
                .map(|context_index| {
                    let (text, truncated) = bounded_inspect_text(lines[context_index]);
                    if truncated {
                        any_line_truncated = true;
                        truncated_lines.insert(context_index + 1);
                    }
                    text
                })
                .collect::<Vec<_>>();
            let context_after = (line_index + 1..after_end)
                .map(|context_index| {
                    let (text, truncated) = bounded_inspect_text(lines[context_index]);
                    if truncated {
                        any_line_truncated = true;
                        truncated_lines.insert(context_index + 1);
                    }
                    text
                })
                .collect::<Vec<_>>();
            let mut match_result = json!({
                "line": line_index + 1,
                "column": column,
                "text": match_text
            });
            if !context_before.is_empty() {
                match_result["context_before"] = json!(context_before.join("\n"));
            }
            if !context_after.is_empty() {
                match_result["context_after"] = json!(context_after.join("\n"));
            }
            matches.push(match_result);
        }
        let mut search_result = json!({
            "pattern": search.pattern,
            "matches": matches
        });
        if search.regex {
            search_result["regex"] = json!(true);
        }
        if !search.case_sensitive {
            search_result["case_sensitive"] = json!(false);
        }
        if search_truncated {
            search_result["truncated"] = json!(true);
        }
        search_results.push(search_result);
    }

    let relative_file = resolved_file
        .strip_prefix(&workspace_root)
        .context("resolved file is outside the selected workspace")?
        .to_string_lossy()
        .replace('\\', "/");
    let mut result = json!({ "file": relative_file });
    if !range_results.is_empty() {
        result["ranges"] = json!(range_results);
    }
    if !search_results.is_empty() {
        result["searches"] = json!(search_results);
    }
    if any_line_truncated || any_search_truncated {
        result["truncated"] = json!(true);
    }
    if !truncated_lines.is_empty() {
        result["truncated_lines"] = json!(truncated_lines);
    }
    Ok(result)
}

fn bounded_inspect_text(line: &str) -> (String, bool) {
    let mut chars = line.chars();
    let text = chars
        .by_ref()
        .take(MAX_INSPECT_LINE_CHARS)
        .collect::<String>();
    (text, chars.next().is_some())
}
