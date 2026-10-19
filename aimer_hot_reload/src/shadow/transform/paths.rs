use super::*;

pub(super) fn path_candidates(path: &[String], module: &[String]) -> Vec<Vec<String>> {
    if path.first().is_some_and(|segment| segment == "crate") {
        return vec![path.to_vec()];
    }
    if path.first().is_some_and(|segment| segment == "self") {
        return vec![
            module
                .iter()
                .cloned()
                .chain(path.iter().skip(1).cloned())
                .collect(),
        ];
    }
    if path.first().is_some_and(|segment| segment == "super") {
        let mut base = module.to_vec();
        let mut index = 0;
        while path.get(index).is_some_and(|segment| segment == "super") {
            if base.len() > 1 {
                base.pop();
            }
            index += 1;
        }
        base.extend_from_slice(&path[index..]);
        return vec![base];
    }
    vec![
        module.iter().cloned().chain(path.iter().cloned()).collect(),
        std::iter::once("crate".to_owned())
            .chain(path.iter().cloned())
            .collect(),
    ]
}

pub(super) fn collect_use_aliases(
    tree: &syn::UseTree,
    prefix: Vec<String>,
    module: &[String],
    aliases: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            collect_use_aliases(&path.tree, next, module, aliases);
        }
        syn::UseTree::Name(name) => {
            let mut target = prefix;
            target.push(name.ident.to_string());
            aliases.insert(name.ident.to_string(), normalize_use_path(target, module));
        }
        syn::UseTree::Rename(rename) => {
            let mut target = prefix;
            target.push(rename.ident.to_string());
            aliases.insert(
                rename.rename.to_string(),
                normalize_use_path(target, module),
            );
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_aliases(item, prefix.clone(), module, aliases);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

pub(super) fn normalize_use_path(path: Vec<String>, module: &[String]) -> Vec<String> {
    if path.first().is_some_and(|segment| segment == "self") {
        module
            .iter()
            .cloned()
            .chain(path.into_iter().skip(1))
            .collect()
    } else {
        path
    }
}

pub(super) fn bound_is_callback(bound: &syn::TypeParamBound) -> bool {
    matches!(bound, syn::TypeParamBound::Trait(trait_bound)
    if trait_bound.path.segments.last().is_some_and(|segment| {
        matches!(segment.ident.to_string().as_str(), "Fn" | "FnMut" | "FnOnce")
    }))
}

pub(super) fn one_type_argument(segment: &syn::PathSegment) -> Option<&syn::Type> {
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

pub(super) fn is_primitive(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "String"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
    )
}

pub(super) fn type_key(module: &[String], name: &str) -> String {
    format!("{}::{name}", module.join("::"))
}

pub(super) fn source_offset(source: &str, location: proc_macro2::LineColumn) -> Option<usize> {
    let mut line_starts = vec![0usize];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(index + 1);
        }
    }
    let line_start = *line_starts.get(location.line.checked_sub(1)?)?;
    let offset = line_start.checked_add(location.column)?;
    (offset <= source.len()).then_some(offset)
}

pub(super) fn compact_tokens(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

pub(super) fn module_directory(file: &Path) -> PathBuf {
    let parent = file.parent().expect("Rust source has a parent");
    match file.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "main.rs" | "mod.rs") => parent.to_owned(),
        _ => parent.join(file.file_stem().expect("Rust source has a stem")),
    }
}

pub(super) fn malformed(path: &Path, error: syn::Error) -> ShadowError {
    ShadowError::new(
        ShadowErrorKind::MalformedSource,
        format!("failed to parse Rust module {}: {error}", path.display()),
    )
}

pub(super) fn io_error(action: &str, path: &Path, error: std::io::Error) -> ShadowError {
    ShadowError::new(
        ShadowErrorKind::Io,
        format!("failed to {action} Rust module {}: {error}", path.display()),
    )
}
