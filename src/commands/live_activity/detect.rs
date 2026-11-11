//! Telling from a `Cargo.toml` whether the project uses live activities.

use toml::{Table, Value};

/// The feature of the `aimer` umbrella crate that turns live activities on.
const FEATURE: &str = "live-activity";

/// Whether `manifest` (the text of a `Cargo.toml`) pulls in live activities:
/// either the `live-activity` feature of `aimer`, or `aimer_live_activity`
/// itself.
///
/// Dependencies are read from `[dependencies]` and every
/// `[target.*.dependencies]`. A manifest that does not parse counts as "no":
/// Cargo reports that on its own, and the project must still build.
pub(super) fn enabled(manifest: &str) -> bool {
    let Ok(root) = manifest.parse::<Table>() else {
        return false;
    };
    dependency_tables(&root).any(|dependencies| {
        dependencies.iter().any(|(name, spec)| {
            is_crate(name, "aimer_live_activity") || (name == "aimer" && has_feature(spec))
        })
    })
}

/// Every dependency table of the manifest.
fn dependency_tables(root: &Table) -> impl Iterator<Item = &Table> {
    let top = root.get("dependencies").and_then(Value::as_table);
    let targets = root
        .get("target")
        .and_then(Value::as_table)
        .into_iter()
        .flat_map(|targets| targets.values())
        .filter_map(|target| target.get("dependencies").and_then(Value::as_table));
    top.into_iter().chain(targets)
}

/// Cargo treats `-` and `_` in a crate name alike.
fn is_crate(name: &str, wanted: &str) -> bool {
    name.replace('-', "_") == wanted
}

/// Whether a dependency specification lists [`FEATURE`].
fn has_feature(spec: &Value) -> bool {
    spec.get("features")
        .and_then(Value::as_array)
        .is_some_and(|features| features.iter().any(|f| f.as_str() == Some(FEATURE)))
}
