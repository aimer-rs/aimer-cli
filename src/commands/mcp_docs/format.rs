use super::*;

pub(super) fn format_trait_signature(name: &str, trait_data: &serde_json::Value) -> String {
    let unsafe_modifier = trait_data
        .get("is_unsafe")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        .then_some("unsafe ")
        .unwrap_or_default();
    let auto_modifier = trait_data
        .get("is_auto")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        .then_some("auto ")
        .unwrap_or_default();
    let generics = trait_data
        .get("generics")
        .and_then(|generics| generics.get("params"))
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, true))
        .unwrap_or_default();
    let bounds = trait_data
        .get("bounds")
        .and_then(serde_json::Value::as_array)
        .map(|bounds| {
            bounds
                .iter()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .unwrap_or_default();
    let supertraits = if bounds.is_empty() {
        String::new()
    } else {
        format!(": {bounds}")
    };
    let where_clauses = trait_data
        .get("generics")
        .and_then(|generics| generics.get("where_predicates"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(format_where_predicate)
        .collect::<Vec<_>>();
    let where_clause = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("\nwhere\n    {}", where_clauses.join(",\n    "))
    };

    format!("pub {unsafe_modifier}{auto_modifier}trait {name}{generics}{supertraits}{where_clause}")
}

pub(super) fn format_trait_definition(
    signature: &str,
    methods: &[TraitMethodDocumentation],
) -> String {
    let mut lines = vec![format!("{signature} {{")];
    for method in methods {
        let method_kind = if method.required {
            "Required"
        } else {
            "Provided"
        };
        lines.push(format!("    // {method_kind} method"));
        let mut declaration = method.signature.clone();
        if method.required {
            declaration.push(';');
        } else if declaration.contains('\n') {
            declaration.push_str("\n{");
            declaration.push_str("\n    /* provided method body */\n}");
        } else {
            declaration.push_str(" { /* provided method body */ }");
        }
        lines.extend(declaration.lines().map(|line| format!("    {line}")));
    }
    lines.push("}".to_owned());
    lines.join("\n")
}

pub(super) fn format_method_signature(name: &str, function: &serde_json::Value) -> String {
    let signature = function.get("sig").unwrap_or(&serde_json::Value::Null);
    let generics = function.get("generics").unwrap_or(&serde_json::Value::Null);
    let generic_parameters = generics
        .get("params")
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, false))
        .unwrap_or_default();
    let header = function.get("header").unwrap_or(&serde_json::Value::Null);
    let mut modifiers = Vec::new();
    for (field, keyword) in [
        ("is_const", "const"),
        ("is_async", "async"),
        ("is_unsafe", "unsafe"),
    ] {
        if header
            .get(field)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            modifiers.push(keyword.to_owned());
        }
    }
    if let Some(abi) = header
        .get("abi")
        .and_then(serde_json::Value::as_str)
        .filter(|abi| !abi.is_empty() && *abi != "Rust")
    {
        modifiers.push(format!("extern \"{abi}\""));
    }
    let modifiers = if modifiers.is_empty() {
        String::new()
    } else {
        format!("{} ", modifiers.join(" "))
    };
    let inputs = signature
        .get("inputs")
        .and_then(serde_json::Value::as_array)
        .map(|inputs| {
            inputs
                .iter()
                .map(format_function_input)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let mut rendered = format!("{modifiers}fn {name}{generic_parameters}({inputs})");
    if let Some(output) = signature.get("output").filter(|output| !output.is_null()) {
        rendered.push_str(" -> ");
        rendered.push_str(&format_type(output));
    }
    let where_clauses = format_where_clauses(generics);
    if !where_clauses.is_empty() {
        rendered.push_str("\nwhere\n    ");
        rendered.push_str(&where_clauses.join(",\n    "));
    }
    rendered
}

pub(super) fn format_function_input(input: &serde_json::Value) -> String {
    let Some(parts) = input.as_array() else {
        return format_type(input);
    };
    let Some(name) = parts.first().and_then(serde_json::Value::as_str) else {
        return parts
            .get(1)
            .map(format_type)
            .unwrap_or_else(|| "_".to_owned());
    };
    let Some(ty) = parts.get(1) else {
        return name.to_owned();
    };
    if name == "self" {
        if let Some(reference) = ty.get("borrowed_ref") {
            let lifetime = reference
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .filter(|lifetime| !lifetime.is_empty())
                .map(|lifetime| format!("{lifetime} "))
                .unwrap_or_default();
            let mutable = reference
                .get("is_mutable")
                .or_else(|| reference.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            return format!("&{lifetime}{}self", if mutable { "mut " } else { "" });
        }
        if type_is_self(ty) {
            return "self".to_owned();
        }
        return format!("self: {}", format_type(ty));
    }
    format!("{name}: {}", format_type(ty))
}

pub(super) fn type_is_self(ty: &serde_json::Value) -> bool {
    ty.get("generic").and_then(serde_json::Value::as_str) == Some("Self")
}

pub(super) fn format_generic_params(params: &[serde_json::Value], include_bounds: bool) -> String {
    let rendered = params
        .iter()
        .map(|param| format_generic_param(param, include_bounds))
        .filter(|param| !param.is_empty())
        .collect::<Vec<_>>();
    if rendered.is_empty() {
        String::new()
    } else {
        format!("<{}>", rendered.join(", "))
    }
}

pub(super) fn format_generic_param(param: &serde_json::Value, include_bounds: bool) -> String {
    let Some(name) = param.get("name").and_then(serde_json::Value::as_str) else {
        return String::new();
    };
    let kind = param.get("kind").unwrap_or(&serde_json::Value::Null);
    if let Some(lifetime) = kind.get("lifetime") {
        let bounds = lifetime
            .get("outlives")
            .and_then(serde_json::Value::as_array)
            .map(|bounds| {
                bounds
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .unwrap_or_default();
        return if include_bounds && !bounds.is_empty() {
            format!("{name}: {bounds}")
        } else {
            name.to_owned()
        };
    }
    if let Some(type_param) = kind.get("type") {
        let bounds = type_param
            .get("bounds")
            .and_then(serde_json::Value::as_array)
            .map(|bounds| {
                bounds
                    .iter()
                    .map(format_generic_bound)
                    .filter(|bound| !bound.is_empty())
                    .collect::<Vec<_>>()
                    .join(" + ")
            })
            .unwrap_or_default();
        let default = type_param
            .get("default")
            .filter(|default| !default.is_null())
            .map(format_type)
            .unwrap_or_default();
        if include_bounds && !bounds.is_empty() {
            return if default.is_empty() {
                format!("{name}: {bounds}")
            } else {
                format!("{name}: {bounds} = {default}")
            };
        }
        return if include_bounds && !default.is_empty() {
            format!("{name} = {default}")
        } else {
            name.to_owned()
        };
    }
    if let Some(constant) = kind.get("const") {
        let ty = constant
            .get("type")
            .map(format_type)
            .unwrap_or_else(|| "_".to_owned());
        let default = constant
            .get("default")
            .and_then(serde_json::Value::as_str)
            .map(|default| format!(" = {default}"))
            .unwrap_or_default();
        return format!("const {name}: {ty}{default}");
    }
    name.to_owned()
}

pub(super) fn format_where_clauses(generics: &serde_json::Value) -> Vec<String> {
    let mut clauses = Vec::new();
    if let Some(params) = generics.get("params").and_then(serde_json::Value::as_array) {
        for param in params {
            let Some(name) = param.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let kind = param.get("kind").unwrap_or(&serde_json::Value::Null);
            let bounds = if let Some(type_param) = kind.get("type") {
                type_param
                    .get("bounds")
                    .and_then(serde_json::Value::as_array)
                    .map(|bounds| {
                        bounds
                            .iter()
                            .map(format_generic_bound)
                            .filter(|bound| !bound.is_empty())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else if let Some(lifetime) = kind.get("lifetime") {
                lifetime
                    .get("outlives")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            } else {
                Vec::new()
            };
            if !bounds.is_empty() {
                clauses.push(format!("{name}: {}", bounds.join(" + ")));
            }
        }
    }
    if let Some(predicates) = generics
        .get("where_predicates")
        .and_then(serde_json::Value::as_array)
    {
        clauses.extend(predicates.iter().filter_map(format_where_predicate));
    }
    let mut unique = Vec::new();
    for clause in clauses {
        if !unique.contains(&clause) {
            unique.push(clause);
        }
    }
    unique
}

pub(super) fn format_where_predicate(predicate: &serde_json::Value) -> Option<String> {
    let (kind, predicate) = predicate.as_object()?.iter().next()?;
    if kind == "bound_predicate" {
        let target = predicate.get("type").map(format_type)?;
        let bounds = predicate
            .get("bounds")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .map(format_generic_bound)
            .filter(|bound| !bound.is_empty())
            .collect::<Vec<_>>();
        if bounds.is_empty() {
            return None;
        }
        let generic_params = predicate
            .get("generic_params")
            .and_then(serde_json::Value::as_array)
            .map(|params| format_generic_params(params, true))
            .unwrap_or_default();
        let for_prefix = if generic_params.is_empty() {
            String::new()
        } else {
            format!("for{generic_params} ")
        };
        return Some(format!("{for_prefix}{target}: {}", bounds.join(" + ")));
    }
    if kind == "lifetime_predicate" {
        let name = predicate
            .get("lifetime")
            .and_then(serde_json::Value::as_str)?;
        let outlives = predicate
            .get("outlives")
            .and_then(serde_json::Value::as_array)?
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        return (!outlives.is_empty()).then(|| format!("{name}: {}", outlives.join(" + ")));
    }
    if kind == "eq_predicate" {
        let left = predicate.get("lhs").map(format_type)?;
        let right = predicate.get("rhs").map(format_type)?;
        return Some(format!("{left} = {right}"));
    }
    None
}

pub(super) fn format_generic_bound(bound: &serde_json::Value) -> String {
    let Some(bounds) = bound.as_object() else {
        return String::new();
    };
    if let Some(trait_bound) = bounds.get("trait_bound") {
        return format_poly_trait(
            trait_bound,
            matches!(
                trait_bound
                    .get("modifier")
                    .and_then(serde_json::Value::as_str),
                Some("maybe") | Some("maybe_const")
            ),
        );
    }
    if bounds.contains_key("trait") {
        return format_poly_trait(bound, false);
    }
    bounds
        .get("outlives")
        .and_then(serde_json::Value::as_str)
        .or_else(|| bounds.get("lifetime").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn format_generic_args(args: &serde_json::Value) -> String {
    if let Some(angle) = args.get("angle_bracketed") {
        let mut values = angle
            .get("args")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_generic_arg)
            .filter(|arg| !arg.is_empty())
            .collect::<Vec<_>>();
        values.extend(
            angle
                .get("bindings")
                .or_else(|| angle.get("constraints"))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .map(format_associated_binding)
                .filter(|binding| !binding.is_empty()),
        );
        if values.is_empty() {
            String::new()
        } else {
            format!("<{}>", values.join(", "))
        }
    } else if let Some(parenthesized) = args.get("parenthesized") {
        let inputs = parenthesized
            .get("inputs")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_type)
            .collect::<Vec<_>>()
            .join(", ");
        let output = parenthesized
            .get("output")
            .filter(|output| !output.is_null())
            .map(|output| format!(" -> {}", format_type(output)))
            .unwrap_or_default();
        format!("({inputs}){output}")
    } else {
        String::new()
    }
}

pub(super) fn format_generic_arg(arg: &serde_json::Value) -> String {
    let Some((kind, value)) = arg.as_object().and_then(|args| args.iter().next()) else {
        return String::new();
    };
    match kind.as_str() {
        "type" => format_type(value),
        "lifetime" => value.as_str().unwrap_or_default().to_owned(),
        "const" => value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string()),
        "infer" => "_".to_owned(),
        _ => String::new(),
    }
}

pub(super) fn format_associated_binding(binding: &serde_json::Value) -> String {
    let Some(name) = binding.get("name").and_then(serde_json::Value::as_str) else {
        return String::new();
    };
    if let Some(value) = binding.get("type") {
        return format!("{name} = {}", format_type(value));
    }
    if let Some(binding) = binding.get("binding") {
        let bounds = binding
            .get("constraints")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .map(format_generic_bound)
            .filter(|bound| !bound.is_empty())
            .collect::<Vec<_>>();
        if !bounds.is_empty() {
            return format!("{name}: {}", bounds.join(" + "));
        }
    }
    String::new()
}

pub(super) fn format_type(ty: &serde_json::Value) -> String {
    let Some((kind, value)) = ty.as_object().and_then(|types| types.iter().next()) else {
        return "_".to_owned();
    };
    match kind.as_str() {
        "resolved_path" => {
            let path = value
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_");
            let args = value
                .get("args")
                .map(format_generic_args)
                .unwrap_or_default();
            format!("{path}{args}")
        }
        "generic" | "primitive" => value.as_str().unwrap_or("_").to_owned(),
        "borrowed_ref" => {
            let lifetime = value
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .filter(|lifetime| !lifetime.is_empty())
                .map(|lifetime| format!("{lifetime} "))
                .unwrap_or_default();
            let mutable = value
                .get("is_mutable")
                .or_else(|| value.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let pointee = value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            format!("&{lifetime}{}{pointee}", if mutable { "mut " } else { "" })
        }
        "raw_pointer" => {
            let mutable = value
                .get("is_mutable")
                .or_else(|| value.get("mutable"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let pointee = value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            format!("*{} {pointee}", if mutable { "mut" } else { "const" })
        }
        "tuple" => format!(
            "({})",
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "slice" => format!(
            "[{}]",
            value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned())
        ),
        "array" => format!(
            "[{}; {}]",
            value
                .get("type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned()),
            value
                .get("len")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_")
        ),
        "never" => "!".to_owned(),
        "infer" => "_".to_owned(),
        "qualified_path" => {
            let name = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("_");
            let self_type = value
                .get("self_type")
                .map(format_type)
                .unwrap_or_else(|| "_".to_owned());
            let trait_type = value
                .get("trait")
                .or_else(|| value.get("trait_"))
                .filter(|trait_| !trait_.is_null())
                .map(format_path)
                .filter(|path| !path.is_empty() && path != "_")
                .map(|path| format!(" as {path}"))
                .unwrap_or_default();
            let args = value
                .get("args")
                .map(format_generic_args)
                .unwrap_or_default();
            if trait_type.is_empty() {
                format!("{self_type}::{name}{args}")
            } else {
                format!("<{self_type}{trait_type}>::{name}{args}")
            }
        }
        "dyn_trait" => {
            let traits = value
                .get("traits")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ");
            format!("dyn {traits}")
        }
        "impl_trait" => format!(
            "impl {}",
            value
                .as_array()
                .into_iter()
                .flatten()
                .map(format_generic_bound)
                .filter(|bound| !bound.is_empty())
                .collect::<Vec<_>>()
                .join(" + ")
        ),
        _ => "_".to_owned(),
    }
}

pub(super) fn format_poly_trait(trait_bound: &serde_json::Value, maybe: bool) -> String {
    let path = trait_bound.get("trait").unwrap_or(&serde_json::Value::Null);
    let generic_params = trait_bound
        .get("generic_params")
        .and_then(serde_json::Value::as_array)
        .map(|params| format_generic_params(params, true))
        .unwrap_or_default();
    let for_prefix = if generic_params.is_empty() {
        String::new()
    } else {
        format!("for{generic_params} ")
    };
    format!(
        "{for_prefix}{}{path}",
        if maybe { "?" } else { "" },
        path = format_path(path)
    )
}

pub(super) fn format_path(path: &serde_json::Value) -> String {
    let name = path
        .get("path")
        .and_then(serde_json::Value::as_str)
        .or_else(|| path.as_str())
        .unwrap_or("_");
    let args = path
        .get("args")
        .filter(|args| !args.is_null())
        .map(format_generic_args)
        .unwrap_or_default();
    format!("{name}{args}")
}
