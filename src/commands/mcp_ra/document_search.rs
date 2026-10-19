use super::*;

pub async fn search_text(
    root: &Path,
    query: &str,
    file_glob: Option<&str>,
    case_sensitive: bool,
    regex: bool,
    limit: usize,
) -> anyhow::Result<Value> {
    let query = query.trim();
    if query.is_empty() {
        bail!("query must not be empty");
    }
    if query.len() > 512 {
        bail!("query must be at most 512 bytes");
    }
    if !(1..=MAX_RESULTS).contains(&limit) {
        bail!("limit must be between 1 and {MAX_RESULTS}");
    }
    let candidates = match Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .await
    {
        Ok(output) if output.status.success() => output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| root.join(String::from_utf8_lossy(path).as_ref()))
            .collect::<Vec<_>>(),
        _ => {
            let mut files = Vec::new();
            collect_search_files(root, root, &mut files)?;
            files
        }
    };

    let pattern = if regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let expression = RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .with_context(|| format!("invalid regex pattern '{query}'"))?;
    let mut results = Vec::new();
    let mut truncated = false;
    for path in candidates {
        if path_glob_matches(file_glob, root, &path) == Some(false) {
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(root) {
            continue;
        }
        let Ok(metadata) = tokio::fs::metadata(&canonical).await else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            continue;
        }
        let Ok(contents) = tokio::fs::read_to_string(&canonical).await else {
            continue;
        };
        for (line_index, line) in contents.lines().enumerate() {
            if let Some(found) = expression.find(line) {
                if results.len() == limit {
                    truncated = true;
                    break;
                }
                let column = line[..found.start()].chars().count() + 1;
                results.push(json!({
                    "file": canonical.strip_prefix(root)?.to_string_lossy().replace('\\', "/"),
                    "line": line_index + 1,
                    "column": column,
                    "text": line.trim().chars().take(500).collect::<String>()
                }));
            }
        }
        if truncated {
            break;
        }
    }
    let mut result = json!({ "query": query, "results": results });
    if truncated {
        result["truncated"] = Value::Bool(true);
    }
    Ok(result)
}

/// Read selected 1-based line ranges and search one project file in a single
/// bounded response. Search patterns are evaluated once per source line.

pub(crate) fn path_glob_matches(pattern: Option<&str>, root: &Path, path: &Path) -> Option<bool> {
    let pattern = pattern?;
    let relative = path
        .strip_prefix(root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    Some(simple_glob(pattern, &relative))
}

pub(super) fn simple_glob(pattern: &str, value: &str) -> bool {
    fn matches(
        pattern: &[char],
        value: &[char],
        pi: usize,
        vi: usize,
        memo: &mut HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(result) = memo.get(&(pi, vi)) {
            return *result;
        }
        let result = if pi == pattern.len() {
            vi == value.len()
        } else if pattern[pi] == '*' {
            let double = pattern.get(pi + 1) == Some(&'*');
            let next = pi + if double { 2 } else { 1 };
            if double
                && pattern.get(next) == Some(&'/')
                && matches(pattern, value, next + 1, vi, memo)
            {
                true
            } else {
                let mut result = false;
                let mut end = vi;
                while end <= value.len() {
                    if matches(pattern, value, next, end, memo) {
                        result = true;
                        break;
                    }
                    if end == value.len() || (!double && value[end] == '/') {
                        break;
                    }
                    end += 1;
                }
                result
            }
        } else if pattern[pi] == '?' {
            vi < value.len() && value[vi] != '/' && matches(pattern, value, pi + 1, vi + 1, memo)
        } else {
            vi < value.len()
                && pattern[pi] == value[vi]
                && matches(pattern, value, pi + 1, vi + 1, memo)
        };
        memo.insert((pi, vi), result);
        result
    }

    let pattern = pattern.replace('\\', "/").chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    matches(&pattern, &value, 0, 0, &mut HashMap::new())
}

pub(super) fn collect_search_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("reading directory '{}' ", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.file_type()?;
        if metadata.is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            if name == ".git" || name == "target" || name == ".aimer" || name.starts_with('.') {
                continue;
            }
            collect_search_files(root, &path, output)?;
        } else if metadata.is_file() && path.starts_with(root) {
            output.push(path);
        }
    }
    Ok(())
}
