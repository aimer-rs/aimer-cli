use super::*;
use quote::ToTokens;
use std::collections::HashMap;
use syn::spanned::Spanned;
use syn::visit::Visit;

const MAX_IMPLEMENTATION_PAGE_SIZE: usize = 200;
const MAX_IMPLEMENTATION_OFFSET: usize = 1_000_000;
const MAX_SOURCE_SCAN_FILES: usize = 50_000;

pub(super) fn implementation_page(
    root: &Path,
    raw: &Value,
    trait_name: Option<&str>,
    limit: usize,
    offset: usize,
    file_glob: Option<&str>,
    include_tests: bool,
) -> anyhow::Result<Value> {
    anyhow::ensure!(
        (1..=MAX_IMPLEMENTATION_PAGE_SIZE).contains(&limit),
        "limit must be between 1 and {MAX_IMPLEMENTATION_PAGE_SIZE}"
    );
    anyhow::ensure!(
        offset <= MAX_IMPLEMENTATION_OFFSET,
        "offset must be between 0 and {MAX_IMPLEMENTATION_OFFSET}"
    );

    let engine_results = raw
        .get("results")
        .and_then(Value::as_array)
        .or_else(|| raw.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let engine_complete = raw.get("complete").and_then(Value::as_bool).unwrap_or(true);
    let mut engine_normalization_complete = true;
    let mut results = Vec::new();
    for item in engine_results {
        let locations = match normalize_implementation_location(root, item, trait_name) {
            Ok(locations) => locations,
            Err(_) => {
                engine_normalization_complete = false;
                continue;
            }
        };
        for result in locations {
            if !include_tests && result["in_test_cfg"].as_bool() == Some(true) {
                continue;
            }
            if crate::commands::mcp_ra::path_glob_matches(
                file_glob,
                root,
                &root.join(result["file"].as_str().unwrap_or_default()),
            ) == Some(false)
            {
                continue;
            }
            results.push(result);
        }
    }
    let (source_results, scan_complete, scan_reason) = if trait_name.is_some() {
        scan_project_implementations(root, trait_name, include_tests)?
    } else {
        (Vec::new(), true, None)
    };
    results.extend(source_results);
    results.sort_by(|left, right| {
        left["file"]
            .as_str()
            .cmp(&right["file"].as_str())
            .then_with(|| left["line"].as_u64().cmp(&right["line"].as_u64()))
            .then_with(|| left["header"].as_str().cmp(&right["header"].as_str()))
    });
    results.dedup_by(|left, right| {
        left["file"] == right["file"]
            && left["line"] == right["line"]
            && left["header"] == right["header"]
    });

    let total_results = results.len();
    let page = results
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let complete = engine_complete && engine_normalization_complete && scan_complete;
    let mut result = json!({
        "results": page,
        "limit": limit,
        "offset": offset,
        "total_results": total_results,
        "truncated": offset.saturating_add(limit) < total_results,
        "complete": complete
    });
    if !complete {
        let reason = if !engine_complete {
            raw.get("reason")
                .cloned()
                .unwrap_or_else(|| json!("engine_returned_partial_results"))
        } else if !engine_normalization_complete {
            json!("engine_result_unmapped")
        } else {
            json!(scan_reason.unwrap_or("source_scan_incomplete"))
        };
        result["reason"] = reason;
        if !scan_complete || !engine_normalization_complete {
            let mut additional_reasons = Vec::new();
            if !engine_normalization_complete {
                additional_reasons.push(json!("engine_result_unmapped"));
            }
            if !scan_complete {
                additional_reasons.push(json!(scan_reason.unwrap_or("source_scan_incomplete")));
            }
            result["additional_reasons"] = json!(additional_reasons);
        }
        if let Some(engine_limit) = raw.get("engine_limit") {
            result["engine_limit"] = engine_limit.clone();
        }
    }
    Ok(result)
}

fn scan_project_implementations(
    root: &Path,
    trait_name: Option<&str>,
    include_tests: bool,
) -> anyhow::Result<(Vec<Value>, bool, Option<&'static str>)> {
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving Rust project root '{}'", root.display()))?;
    let mut pending = vec![root.clone()];
    let mut files = Vec::new();
    let mut complete = true;
    let mut reason = None;
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            complete = false;
            reason.get_or_insert("source_scan_read_error");
            continue;
        };
        let mut collected_entries = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => collected_entries.push(entry),
                Err(_) => {
                    complete = false;
                    reason.get_or_insert("source_scan_read_error");
                }
            }
        }
        let mut entries = collected_entries;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                complete = false;
                reason.get_or_insert("source_scan_read_error");
                continue;
            };
            if kind.is_dir() {
                let name = entry.file_name();
                if !matches!(
                    name.to_str(),
                    Some(".git" | "target" | "build" | "node_modules")
                ) {
                    pending.push(path);
                }
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                if files.len() == MAX_SOURCE_SCAN_FILES {
                    complete = false;
                    reason.get_or_insert("source_scan_file_limit");
                    break;
                }
                files.push(path);
            }
        }
        if !complete && reason == Some("source_scan_file_limit") {
            break;
        }
    }
    files.sort();
    let test_include_map = if include_tests {
        HashMap::new()
    } else {
        included_test_cfg_map(&root, &files)
    };

    let mut results = Vec::new();
    for path in files {
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.len() <= 2 * 1024 * 1024 => (),
            Ok(_) => {
                complete = false;
                reason.get_or_insert("source_file_size_limit");
                continue;
            }
            Err(_) => {
                complete = false;
                reason.get_or_insert("source_scan_read_error");
                continue;
            }
        }
        let inherited_test_cfg = test_include_map.get(&path).copied().unwrap_or(false);
        let implementations = match collect_impls(&path, inherited_test_cfg) {
            Ok(implementations) => implementations,
            Err(_) => {
                complete = false;
                reason.get_or_insert("source_parse_error");
                continue;
            }
        };
        let relative = path
            .strip_prefix(&root)
            .context("scanned Rust source is outside the project")?
            .to_string_lossy()
            .replace('\\', "/");
        let package = crate_name_for_file(&root, &path);
        for implementation in implementations.into_iter().filter(|implementation| {
            (include_tests || !implementation.in_test_cfg)
                && trait_name.is_none_or(|name| implementation.trait_name.as_deref() == Some(name))
        }) {
            results.push(implementation_result(
                &relative,
                &path,
                package.as_deref(),
                implementation,
            ));
        }
    }
    Ok((results, complete, reason))
}

fn included_test_cfg_map(root: &Path, files: &[PathBuf]) -> HashMap<PathBuf, bool> {
    let mut included = HashMap::new();
    for source_path in files {
        let Ok(source) = std::fs::read_to_string(source_path) else {
            continue;
        };
        let Ok(syntax) = syn::parse_file(&source) else {
            continue;
        };
        let mut collector = IncludeCollector {
            target_line: None,
            in_test_cfg: false,
            includes: Vec::new(),
        };
        collector.visit_file(&syntax);
        for (relative, in_test_cfg) in collector.includes {
            let candidate = source_path
                .parent()
                .unwrap_or(root)
                .join(relative)
                .canonicalize();
            let Ok(candidate) = candidate else {
                continue;
            };
            if !candidate.starts_with(root) || !candidate.is_file() {
                continue;
            }
            included
                .entry(candidate)
                .and_modify(|existing| *existing &= in_test_cfg)
                .or_insert(in_test_cfg);
        }
    }
    included
}

fn normalize_implementation_location(
    root: &Path,
    location: &Value,
    trait_name: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    let uri = location
        .get("targetUri")
        .or_else(|| location.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("implementation result has no source URI"))?;
    let path = reqwest::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| anyhow::anyhow!("implementation result has an invalid source URI"))?;
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving Rust project root '{}'", root.display()))?;
    let path = path
        .canonicalize()
        .with_context(|| format!("resolving implementation source '{}'", path.display()))?;
    let relative = path
        .strip_prefix(&root)
        .with_context(|| {
            format!(
                "implementation source '{}' is outside the project",
                path.display()
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let range = location
        .get("targetSelectionRange")
        .or_else(|| location.get("range"))
        .or_else(|| location.get("targetRange"));
    let line = range
        .and_then(|range| range.pointer("/start/line"))
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize
        + 1;

    if let Some(includes) = include_targets_at(&root, &path, line)? {
        let mut included_results = Vec::new();
        for (included_path, inherited_test_cfg) in includes {
            let included_relative = included_path
                .strip_prefix(&root)
                .context("included implementation source is outside the project")?
                .to_string_lossy()
                .replace('\\', "/");
            let package = crate_name_for_file(&root, &included_path);
            let implementations = collect_impls(&included_path, inherited_test_cfg)?;
            for implementation in implementations.into_iter().filter(|implementation| {
                trait_name.is_none_or(|name| implementation.trait_name.as_deref() == Some(name))
            }) {
                included_results.push(implementation_result(
                    &included_relative,
                    &included_path,
                    package.as_deref(),
                    implementation,
                ));
            }
        }
        if !included_results.is_empty() {
            return Ok(included_results);
        }
    }

    let package = crate_name_for_file(&root, &path);
    let implementations = collect_impls(&path, false)?;
    let matched = implementations
        .iter()
        .filter(|implementation| {
            trait_name.is_none_or(|name| implementation.trait_name.as_deref() == Some(name))
        })
        .collect::<Vec<_>>();
    let implementation = matched
        .iter()
        .find(|implementation| implementation.start_line <= line && line <= implementation.end_line)
        .or_else(|| {
            matched
                .iter()
                .min_by_key(|implementation| implementation.start_line.abs_diff(line))
        });
    if let Some(implementation) = implementation {
        return Ok(vec![implementation_result(
            &relative,
            &path,
            package.as_deref(),
            (*implementation).clone(),
        )]);
    }

    let source = std::fs::read_to_string(&path)
        .with_context(|| format!("reading implementation source '{}'", path.display()))?;
    let fallback_line = source
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default();
    let name = fallback_line.trim().to_owned();
    Ok(vec![json!({
        "name": name,
        "container": name,
        "header": name,
        "crate": package,
        "file": relative,
        "line": line,
        "in_test_cfg": path_has_test_component(&path)
    })])
}

#[derive(Clone)]
struct ImplInfo {
    start_line: usize,
    end_line: usize,
    name: String,
    header: String,
    trait_name: Option<String>,
    in_test_cfg: bool,
}

fn collect_impls(path: &Path, inherited_test_cfg: bool) -> anyhow::Result<Vec<ImplInfo>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading Rust implementation source '{}'", path.display()))?;
    let syntax = syn::parse_file(&source)
        .with_context(|| format!("parsing Rust implementation source '{}'", path.display()))?;
    let mut collector = ImplCollector {
        in_test_cfg: inherited_test_cfg || path_has_test_component(path),
        implementations: Vec::new(),
    };
    collector.visit_file(&syntax);
    Ok(collector.implementations)
}

struct ImplCollector {
    in_test_cfg: bool,
    implementations: Vec<ImplInfo>,
}

impl<'ast> Visit<'ast> for ImplCollector {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let previous = self.in_test_cfg;
        self.in_test_cfg |= attrs_have_test_cfg(&item.attrs);
        syn::visit::visit_item_mod(self, item);
        self.in_test_cfg = previous;
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let trait_name = item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .map(|segment| segment.ident.to_string());
        let header = impl_header(item);
        let name = compact_tokens(&item.self_ty.to_token_stream());
        let start_line = item.impl_token.span().start().line;
        let end_line = item.span().end().line.max(start_line);
        self.implementations.push(ImplInfo {
            start_line,
            end_line,
            name,
            header,
            trait_name,
            in_test_cfg: self.in_test_cfg || attrs_have_test_cfg(&item.attrs),
        });
        syn::visit::visit_item_impl(self, item);
    }
}

fn impl_header(item: &syn::ItemImpl) -> String {
    let generics = compact_tokens(&item.generics.to_token_stream());
    let where_clause = item
        .generics
        .where_clause
        .as_ref()
        .map(|clause| format!(" {}", compact_tokens(&clause.to_token_stream())))
        .unwrap_or_default();
    let self_type = compact_tokens(&item.self_ty.to_token_stream());
    match &item.trait_ {
        Some((negative, trait_path, _)) => {
            let negative = negative
                .as_ref()
                .map(|token| token.to_token_stream().to_string())
                .unwrap_or_default();
            format!(
                "impl{generics} {negative}{} for {self_type}{where_clause}",
                compact_tokens(&trait_path.to_token_stream())
            )
        }
        None => format!("impl{generics} {self_type}{where_clause}"),
    }
}

fn compact_tokens(tokens: &proc_macro2::TokenStream) -> String {
    tokens
        .to_string()
        .replace(" ::", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace(" & ", "&")
        .replace(" * ", "*")
}

fn attrs_have_test_cfg(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        let Ok(arguments) = attribute.parse_args_with(
            syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        if attribute.path().is_ident("cfg") {
            arguments.iter().any(|meta| meta_contains_test(meta, false))
        } else if attribute.path().is_ident("cfg_attr") {
            arguments
                .first()
                .is_some_and(|condition| meta_contains_test(condition, false))
        } else {
            false
        }
    })
}

fn meta_contains_test(meta: &syn::Meta, negated: bool) -> bool {
    match meta {
        syn::Meta::Path(path) => !negated && path.is_ident("test"),
        syn::Meta::List(list) => {
            let Ok(nested) = list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return false;
            };
            let nested_negated = negated ^ list.path.is_ident("not");
            nested
                .iter()
                .any(|item| meta_contains_test(item, nested_negated))
        }
        syn::Meta::NameValue(_) => false,
    }
}

fn include_targets_at(
    root: &Path,
    path: &Path,
    line: usize,
) -> anyhow::Result<Option<Vec<(PathBuf, bool)>>> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("reading source file '{}'", path.display()))?;
    let Some(line_text) = source.lines().nth(line.saturating_sub(1)) else {
        return Ok(None);
    };
    if !line_text.contains("include!") {
        return Ok(None);
    }
    let syntax = syn::parse_file(&source).ok();
    let mut collector = IncludeCollector {
        target_line: Some(line),
        in_test_cfg: false,
        includes: Vec::new(),
    };
    if let Some(syntax) = syntax {
        collector.visit_file(&syntax);
    }
    let mut includes = Vec::new();
    for (relative, in_test_cfg) in collector.includes {
        let candidate = path.parent().unwrap_or(root).join(relative).canonicalize();
        let Ok(candidate) = candidate else {
            continue;
        };
        if candidate.starts_with(root) && candidate.is_file() {
            includes.push((candidate, in_test_cfg));
        }
    }
    Ok((!includes.is_empty()).then_some(includes))
}

struct IncludeCollector {
    target_line: Option<usize>,
    in_test_cfg: bool,
    includes: Vec<(PathBuf, bool)>,
}

impl<'ast> Visit<'ast> for IncludeCollector {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let previous = self.in_test_cfg;
        self.in_test_cfg |= attrs_have_test_cfg(&item.attrs);
        syn::visit::visit_item_mod(self, item);
        self.in_test_cfg = previous;
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if item.path.is_ident("include")
            && self
                .target_line
                .is_none_or(|line| item.span().start().line == line)
        {
            if let Ok(path) = syn::parse2::<syn::LitStr>(item.tokens.clone()) {
                self.includes
                    .push((PathBuf::from(path.value()), self.in_test_cfg));
            }
        }
        syn::visit::visit_macro(self, item);
    }
}

fn implementation_result(
    relative: &str,
    path: &Path,
    package: Option<&str>,
    implementation: ImplInfo,
) -> Value {
    let in_test_cfg = implementation.in_test_cfg || path_has_test_component(path);
    json!({
        "name": implementation.name,
        "container": implementation.name,
        "header": implementation.header,
        "crate": package,
        "file": relative,
        "line": implementation.start_line,
        "in_test_cfg": in_test_cfg
    })
}

fn crate_name_for_file(root: &Path, file: &Path) -> Option<String> {
    let mut directory = file.parent()?;
    loop {
        if directory.join("Cargo.toml").is_file()
            && let Some(name) = crate::config::parse_cargo_package_name(directory)
        {
            return Some(name);
        }
        if directory == root {
            return None;
        }
        directory = directory.parent()?;
    }
}

fn path_has_test_component(path: &Path) -> bool {
    let directory_is_test = path.components().any(|component| {
        matches!(
            component,
            std::path::Component::Normal(name) if name == "tests" || name == "test"
        )
    });
    let file_is_test = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| {
            stem == "test"
                || stem == "tests"
                || stem.starts_with("test_")
                || stem.ends_with("_tests")
        });
    directory_is_test || file_is_test
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementation_pages_sort_and_label_and_skip_test_cfg_by_default() {
        let root = tempfile::tempdir().unwrap();
        let crate_root = root.path().join("aimer_widget");
        std::fs::create_dir_all(crate_root.join("src")).unwrap();
        std::fs::write(
            crate_root.join("Cargo.toml"),
            "[package]\nname = \"aimer_widget\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            crate_root.join("src/lib.rs"),
            "pub trait Drawable {}\nimpl Drawable for Zed {}\n#[cfg(test)]\nmod tests { impl Drawable for Test {} }\nimpl Drawable for Able {}\n",
        )
        .unwrap();
        let uri = reqwest::Url::from_file_path(crate_root.join("src/lib.rs"))
            .unwrap()
            .to_string();
        let raw = json!({
            "results": [
                {"targetUri": uri, "targetSelectionRange": {"start": {"line": 4}}},
                {"targetUri": uri, "targetSelectionRange": {"start": {"line": 1}}},
                {"targetUri": uri, "targetSelectionRange": {"start": {"line": 3}}}
            ],
            "complete": true
        });

        let result =
            implementation_page(root.path(), &raw, Some("Drawable"), 1, 0, None, false).unwrap();

        assert_eq!(result["total_results"], 2);
        assert_eq!(result["results"][0]["header"], "impl Drawable for Able");
        assert_eq!(result["results"][0]["crate"], "aimer_widget");
        assert_eq!(result["truncated"], true);
    }
}
