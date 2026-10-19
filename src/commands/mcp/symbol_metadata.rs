use super::*;
use syn::spanned::Spanned;
use syn::visit::Visit;

#[derive(Clone)]
pub(super) struct SymbolMetadata {
    pub(super) package: Option<String>,
    pub(super) module_path: Option<String>,
    pub(super) is_reexport: bool,
    pub(super) is_macro: bool,
}

pub(super) fn symbol_metadata(root: &Path, symbol: &Value) -> SymbolMetadata {
    let path = symbol
        .pointer("/location/uri")
        .or_else(|| symbol.get("uri"))
        .and_then(Value::as_str)
        .and_then(|uri| reqwest::Url::parse(uri).ok())
        .and_then(|uri| uri.to_file_path().ok())
        .and_then(|path| path.canonicalize().ok());
    let Some(path) = path else {
        return SymbolMetadata {
            package: None,
            module_path: None,
            is_reexport: false,
            is_macro: false,
        };
    };
    let package_root = package_root_for_file(root, &path);
    let package = package_root
        .as_deref()
        .and_then(|directory| crate::config::parse_cargo_package_name(directory));
    let line = symbol
        .pointer("/location/range/start/line")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize
        + 1;
    let name = symbol
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut module_parts: Vec<String> = package
        .as_deref()
        .map(|name| {
            name.replace('-', "_")
                .split("::")
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if let Some(package_root) = package_root.as_deref() {
        module_parts.extend(file_module_path(package_root, &path));
    }
    let source = std::fs::read_to_string(&path).ok();
    let mut is_reexport = false;
    let mut is_macro = false;
    if let Some(source) = source.as_deref()
        && let Ok(syntax) = syn::parse_file(source)
    {
        let mut visitor = SymbolMetadataVisitor {
            line,
            name,
            inline_modules: Vec::new(),
            is_reexport: false,
            is_macro: false,
        };
        visitor.visit_file(&syntax);
        module_parts.extend(visitor.inline_modules);
        is_reexport = visitor.is_reexport;
        is_macro = visitor.is_macro;
    }
    SymbolMetadata {
        package,
        module_path: (!module_parts.is_empty()).then(|| module_parts.join("::")),
        is_reexport,
        is_macro,
    }
}

fn package_root_for_file(root: &Path, file: &Path) -> Option<PathBuf> {
    let mut directory = file.parent()?;
    loop {
        if directory.join("Cargo.toml").is_file() {
            return Some(directory.to_path_buf());
        }
        if directory == root {
            return None;
        }
        directory = directory.parent()?;
    }
}

fn file_module_path(package_root: &Path, file: &Path) -> Vec<String> {
    let Ok(relative) = file.strip_prefix(package_root.join("src")) else {
        return Vec::new();
    };
    let mut parts = relative
        .components()
        .filter_map(|part| match part {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let Some(last) = parts.last_mut() {
        if last.ends_with(".rs") {
            last.truncate(last.len() - 3);
        }
        if matches!(last.as_str(), "lib" | "main" | "mod") {
            parts.pop();
        }
    }
    parts
}

struct SymbolMetadataVisitor<'a> {
    line: usize,
    name: &'a str,
    inline_modules: Vec<String>,
    is_reexport: bool,
    is_macro: bool,
}

impl<'ast> Visit<'ast> for SymbolMetadataVisitor<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.content.is_some() && span_contains_line(item.span(), self.line) {
            self.inline_modules.push(item.ident.to_string());
        }
        syn::visit::visit_item_mod(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        if span_contains_line(item.span(), self.line)
            && !matches!(&item.vis, syn::Visibility::Inherited)
        {
            self.is_reexport = true;
        }
        syn::visit::visit_item_use(self, item);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if span_contains_line(item.span(), self.line)
            && item.ident.as_ref().is_some_and(|ident| ident == self.name)
        {
            self.is_macro = true;
        }
        syn::visit::visit_item_macro(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.sig.ident == self.name
            && item.attrs.iter().any(|attribute| {
                ["proc_macro", "proc_macro_derive", "proc_macro_attribute"]
                    .iter()
                    .any(|name| attribute.path().is_ident(name))
            })
        {
            self.is_macro = true;
        }
        syn::visit::visit_item_fn(self, item);
    }
}

fn span_contains_line(span: proc_macro2::Span, line: usize) -> bool {
    let start = span.start().line;
    let end = span.end().line.max(start);
    start <= line && line <= end
}
