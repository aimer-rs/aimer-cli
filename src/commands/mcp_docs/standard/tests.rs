use super::*;
use serde_json::json;

fn fixture() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let std = json!({
        "root": 0, "format_version": 61, "crate_version": "1.101.0-nightly\t(hash\t2026-10-01)",
        "index": {
            "0": {"crate_id": 0, "name": "std", "docs": "standard library", "inner": {"module": {"items": [1, 2]}}},
            "1": {"crate_id": 0, "name": null, "inner": {"use": {"name": "vec", "id": 9, "is_glob": false}}},
            "2": {"crate_id": 0, "name": null, "inner": {"use": {"name": "iter", "id": 10, "is_glob": false}}}
        },
        "paths": {
            "9": {"path": ["alloc", "vec"], "kind": "module"},
            "10": {"path": ["core", "iter"], "kind": "module"}
        }
    });
    let alloc = json!({
        "root": 0, "format_version": 61, "crate_version": "1.101.0-nightly",
        "index": {
            "0": {"crate_id": 0, "name": "alloc", "inner": {"module": {"items": [1]}}},
            "1": {"crate_id": 0, "name": "vec", "inner": {"module": {"items": [2, 5]}}},
            "2": {"crate_id": 0, "name": "Vec", "docs": "A growable vector.", "inner": {"struct": {"impls": [3]}}},
            "3": {"crate_id": 0, "name": null, "inner": {"impl": {"items": [4]}}},
            "4": {"crate_id": 0, "name": "new", "docs": "Creates a new vector.", "inner": {"function": {"has_body": true}}},
            "5": {"crate_id": 0, "name": "internal", "visibility": "crate", "inner": {"module": {"items": [6]}}},
            "6": {"crate_id": 0, "name": "Hidden", "visibility": "public", "docs": "Hidden vector implementation.", "inner": {"struct": {"impls": []}}}
        },
        "paths": {"1": {"path": ["alloc", "vec"], "kind": "module"}}
    });
    let core = json!({
        "root": 0, "format_version": 61, "crate_version": "1.101.0-nightly",
        "index": {
            "0": {"crate_id": 0, "name": "core", "inner": {"module": {"items": [1]}}},
            "1": {"crate_id": 0, "name": "iter", "inner": {"module": {"items": [2]}}},
            "2": {"crate_id": 0, "name": "Iterator", "docs": "An iterator.", "inner": {"trait": {"items": [3]}}},
            "3": {"crate_id": 0, "name": "next", "visibility": "default", "docs": "Advance the iterator.", "inner": {"function": {"has_body": false}}}
        },
        "paths": {"1": {"path": ["core", "iter"], "kind": "module"}}
    });
    for (name, value) in [("std", std), ("alloc", alloc), ("core", core)] {
        fs::write(
            directory.path().join(format!("{name}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    directory
}

#[test]
fn standard_docs_resolve_reexported_types_methods_and_traits() {
    let directory = fixture();
    let index = load_indices(directory.path(), "std", "1.101.0-nightly").unwrap();
    assert_eq!(
        index.select_item(None).unwrap().docs.as_deref(),
        Some("standard library")
    );
    let vector = index.select_item(Some("std::vec::Vec")).unwrap();
    assert_eq!(vector.path, "std::vec::Vec");
    assert_eq!(vector.kind, "struct");
    assert_eq!(vector.docs.as_deref(), Some("A growable vector."));
    assert_eq!(
        index
            .select_item(Some("vec::Vec::new"))
            .unwrap()
            .docs
            .as_deref(),
        Some("Creates a new vector.")
    );
    let iterator = index.select_item(Some("iter::Iterator")).unwrap();
    let details = iterator.trait_details.unwrap();
    assert_eq!(details.methods.len(), 1);
    assert_eq!(details.methods[0].name, "next");
    assert!(details.methods[0].required);
    assert_eq!(
        details.methods[0].documentation.as_deref(),
        Some("Advance the iterator.")
    );
    assert!(index.select_item(Some("vec::Missing")).is_err());
}

#[test]
fn search_uses_public_std_paths_for_external_docs_and_respects_limit() {
    let directory = fixture();
    let index = load_indices(directory.path(), "std", "1.101.0-nightly").unwrap();
    let results = index.search_items("growable", 1).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].item_path, "std::vec::Vec");
    assert_eq!(results[0].item_kind, "struct");
    assert_eq!(results[0].summary.as_deref(), Some("A growable vector."));
    assert_eq!(index.search_items("vector", 1).unwrap().len(), 1);
    assert!(index.search_items("Hidden", 100).unwrap().is_empty());
    assert!(index.select_item(Some("vec::internal::Hidden")).is_err());
}

#[test]
fn glob_reexports_keep_child_paths_and_do_not_duplicate_the_module() {
    let directory = fixture();
    let path = directory.path().join("std.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["index"]["1"]["inner"]["use"]["is_glob"] = true.into();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let index = load_indices(directory.path(), "std", "1.101.0-nightly").unwrap();
    assert_eq!(index.select_item(Some("Vec")).unwrap().path, "std::Vec");
    assert_eq!(
        index
            .search_items("std", 100)
            .unwrap()
            .iter()
            .filter(|item| item.item_path == "std")
            .count(),
        1
    );
}

#[test]
fn toolchain_docs_reject_missing_dependencies_and_version_mismatches() {
    let directory = fixture();
    assert!(load_indices(directory.path(), "std", "1.99.0").is_err());
    fs::remove_file(directory.path().join("alloc.json")).unwrap();
    assert!(load_indices(directory.path(), "std", "1.101.0-nightly").is_err());
}

#[test]
fn toolchain_names_cannot_be_paths_or_options() {
    for value in ["", "../nightly", "--help", "nightly/foo", "nightly\n"] {
        assert!(validate_toolchain(value).is_err());
    }
    for value in [
        "nightly",
        "1.99.0",
        "nightly-2026-10-01",
        "nightly-aarch64-apple-darwin",
    ] {
        assert!(validate_toolchain(value).is_ok());
    }
    assert!(is_toolchain_crate("std"));
    assert!(!is_toolchain_crate("serde"));
}
