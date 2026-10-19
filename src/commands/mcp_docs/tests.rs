use super::*;

const DOCS_JSON: &str = r#"{
        "root": 0,
        "format_version": 1,
        "crate_version": "1.2.3",
        "index": {
            "0": {
                "crate_id": 0,
                "name": "demo",
                "docs": "crate docs",
                "inner": { "module": { "is_crate": true, "items": [1] } }
            },
            "1": {
                "crate_id": 0,
                "name": "Guide",
                "docs": "guide docs",
                "inner": { "module": { "is_crate": false, "items": [] } }
            }
        }
    }"#;

fn client_with_cache() -> (tempfile::TempDir, tempfile::TempDir, DocsRsClient) {
    let project = tempfile::tempdir().unwrap();
    let project_root = project.path().join("member");
    fs::create_dir_all(&project_root).unwrap();
    fs::write(
            project.path().join("Cargo.lock"),
            "[[package]]\nname = \"demo\"\nversion = \"1.2.3\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )
        .unwrap();

    let cache = tempfile::tempdir().unwrap();
    let mut client = DocsRsClient::new(project_root).unwrap();
    client.cache_root = cache.path().to_path_buf();
    (project, cache, client)
}

fn cached_arguments() -> GetCrateDocsArguments {
    GetCrateDocsArguments {
        crate_name: "demo".to_owned(),
        item_path: Some("Guide".to_owned()),
        version: None,
        refresh: false,
    }
}

fn get_cached_document(client: &DocsRsClient) -> CrateDocumentation {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                client.get_document(cached_arguments()),
            )
            .await
            .expect("a cache hit should not make a network request")
        })
        .expect("cached documentation should parse")
}

#[test]
fn trait_lookup_returns_definition_required_status_and_method_documentation() {
    let json = serde_json::json!({
        "root": 0,
        "format_version": 1,
        "crate_version": "1.2.3",
        "index": {
            "0": {
                "crate_id": 0,
                "name": "demo",
                "docs": "crate docs",
                "inner": { "module": { "items": [1] } }
            },
            "1": {
                "crate_id": 0,
                "name": "Deserialize",
                "docs": "A data structure that can be deserialized.\n\nExpanded trait description.",
                "inner": {
                    "trait": {
                        "is_auto": false,
                        "is_unsafe": false,
                        "is_dyn_compatible": false,
                        "items": [2],
                        "generics": {
                            "params": [{
                                "name": "'de",
                                "kind": { "lifetime": { "outlives": [] } }
                            }],
                            "where_predicates": []
                        },
                        "bounds": [{
                            "trait_bound": {
                                "trait": { "path": "Sized", "id": 3, "args": null },
                                "generic_params": [],
                                "modifier": "none"
                            }
                        }]
                    }
                }
            },
            "2": {
                "crate_id": 0,
                "name": "deserialize",
                "docs": "Deserialize this value from the given deserializer.\n\nSee the implementation guide for details.",
                "inner": {
                    "function": {
                        "sig": {
                            "inputs": [["deserializer", { "generic": "D" }]],
                            "output": {
                                "resolved_path": {
                                    "path": "Result",
                                    "id": 4,
                                    "args": {
                                        "angle_bracketed": {
                                            "args": [
                                                { "type": { "generic": "Self" } },
                                                { "type": {
                                                    "qualified_path": {
                                                        "name": "Error",
                                                        "args": null,
                                                        "self_type": { "generic": "D" },
                                                        "trait": null
                                                    }
                                                } }
                                            ],
                                            "bindings": []
                                        }
                                    }
                                }
                            }
                        },
                        "generics": {
                            "params": [{
                                "name": "D",
                                "kind": { "type": {
                                    "bounds": [{
                                        "trait_bound": {
                                            "trait": {
                                                "path": "Deserializer",
                                                "id": 5,
                                                "args": {
                                                    "angle_bracketed": {
                                                        "args": [{ "lifetime": "'de" }],
                                                        "bindings": []
                                                    }
                                                }
                                            },
                                            "generic_params": [],
                                            "modifier": "none"
                                        }
                                    }],
                                    "default": null
                                } }
                            }],
                            "where_predicates": []
                        },
                        "header": {
                            "is_const": false,
                            "is_unsafe": false,
                            "is_async": false,
                            "abi": "Rust"
                        },
                        "has_body": false
                    }
                }
            }
        }
    });
    let rustdoc = parse_json_index(&serde_json::to_vec(&json).unwrap(), "1.2.3").unwrap();
    let selected = rustdoc.select_item(Some("Deserialize")).unwrap();
    let response = serde_json::to_value(CrateDocumentation {
        crate_name: "demo".to_owned(),
        version: "1.2.3".to_owned(),
        item_path: selected.path,
        item_kind: selected.kind,
        documentation: selected.docs,
        trait_details: selected.trait_details,
        docs_url: "https://docs.rs/demo/1.2.3/demo".to_owned(),
        source: "cache",
        cache_warning: None,
    })
    .unwrap();

    assert_eq!(response["item_kind"], "trait");
    assert_eq!(
        response["documentation"],
        "A data structure that can be deserialized.\n\nExpanded trait description."
    );
    assert_eq!(
        response["trait_details"]["signature"],
        "pub trait Deserialize<'de>: Sized"
    );
    assert_eq!(
        response["trait_details"]["definition"],
        concat!(
            "pub trait Deserialize<'de>: Sized {\n",
            "    // Required method\n",
            "    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\n",
            "    where\n",
            "        D: Deserializer<'de>;\n",
            "}"
        )
    );
    assert_eq!(
        response["trait_details"]["methods"][0]["name"],
        "deserialize"
    );
    assert_eq!(response["trait_details"]["methods"][0]["required"], true);
    assert_eq!(
        response["trait_details"]["methods"][0]["documentation"],
        "Deserialize this value from the given deserializer.\n\nSee the implementation guide for details."
    );
}

#[test]
fn offline_lookup_reads_zstd_cache_and_resolves_version_from_parent_lockfile() {
    let (_project, _cache, client) = client_with_cache();
    let gzip_path = client.cache_path("demo", "1.2.3");
    fs::create_dir_all(gzip_path.parent().unwrap()).unwrap();
    let zstd_path = gzip_path.with_file_name("default.json.zst");
    fs::write(
        &zstd_path,
        compress_zstd_json(DOCS_JSON.as_bytes()).unwrap(),
    )
    .unwrap();

    let document = get_cached_document(&client);

    assert_eq!(document.source, "cache");
    assert_eq!(document.version, "1.2.3");
    assert_eq!(document.item_path, "demo::Guide");
    assert_eq!(document.item_kind, "module");
    assert_eq!(document.documentation.as_deref(), Some("guide docs"));
    assert_eq!(document.docs_url, "https://docs.rs/demo/1.2.3/demo");
    assert!(!gzip_path.exists());
}

#[test]
fn offline_lookup_upgrades_gzip_cache_to_zstd_sidecar() {
    let (_project, _cache, client) = client_with_cache();
    let gzip_path = client.cache_path("demo", "1.2.3");
    fs::create_dir_all(gzip_path.parent().unwrap()).unwrap();
    fs::write(
        &gzip_path,
        normalize_compressed_json(DOCS_JSON.as_bytes()).unwrap(),
    )
    .unwrap();

    let document = get_cached_document(&client);
    let zstd_path = gzip_path.with_file_name("default.json.zst");

    assert_eq!(document.source, "cache");
    assert_eq!(document.documentation.as_deref(), Some("guide docs"));
    assert!(zstd_path.is_file());
    assert_eq!(
        decode_json_payload(&fs::read(zstd_path).unwrap()).unwrap(),
        DOCS_JSON.as_bytes()
    );
}

#[test]
#[ignore = "contacts docs.rs over the network"]
fn live_docs_rs_download_is_then_read_from_zstd_cache() {
    let (_project, _cache, client) = client_with_cache();
    let arguments = GetCrateDocsArguments {
        crate_name: "serde".to_owned(),
        item_path: Some("de::Deserialize".to_owned()),
        version: Some("1.0.229".to_owned()),
        refresh: false,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let downloaded = runtime
        .block_on(client.get_document(arguments.clone()))
        .unwrap();
    assert_eq!(downloaded.source, "docs.rs");
    assert!(
        downloaded
            .documentation
            .as_deref()
            .is_some_and(|docs| !docs.trim().is_empty())
    );
    let trait_details = downloaded
        .trait_details
        .as_ref()
        .expect("a trait result should contain the trait declaration");
    assert_eq!(trait_details.signature, "pub trait Deserialize<'de>: Sized");
    assert!(trait_details.definition.contains("// Required method"));
    let deserialize = trait_details
        .methods
        .iter()
        .find(|method| method.name == "deserialize")
        .expect("Deserialize should expose its required deserialize method");
    assert!(deserialize.required);
    assert_eq!(
        deserialize.signature,
        "fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>\nwhere\n    D: Deserializer<'de>"
    );
    assert!(
        deserialize
            .documentation
            .as_deref()
            .is_some_and(|docs| !docs.trim().is_empty())
    );

    let gzip_path = client.cache_path("serde", "1.0.229");
    let zstd_path = gzip_path.with_file_name("default.json.zst");
    assert!(gzip_path.is_file());
    assert!(zstd_path.is_file());

    let cached = runtime.block_on(client.get_document(arguments)).unwrap();
    assert_eq!(cached.source, "cache");
    assert_eq!(cached.item_path, downloaded.item_path);
    assert_eq!(cached.documentation, downloaded.documentation);
    assert_eq!(cached.trait_details, downloaded.trait_details);
}
