use super::*;

const CLI_SAFE_TEXT: &str = concat!(
    "version=1\n",
    "widget_ir_diagnostics=0\n",
    "runtime_fuel_per_call=100000000\n",
    "max_module_bytes=16777216\n",
    "runtime_max_memory_pages=64\n",
    "runtime_max_table_elements=2048\n",
    "runtime_max_call_depth=256\n",
    "protocol_max_chunk_bytes=65536\n",
    "protocol_max_diagnostic_bytes=65536\n",
    "protocol_max_terminal_results=4\n",
    "protocol_io_timeout_ms=10000\n",
    "listener_credential_ttl_ms=28800000\n",
    "listener_max_authentication_failures=4\n",
    "listener_authentication_backoff_ms=1000\n",
    "model_max_document_bytes=32768\n",
    "model_max_state_entries=512\n",
    "model_max_widget_nodes=512\n",
    "model_max_properties=512\n",
    "model_max_widget_depth=32\n",
    "generation_max_retired=4\n",
    "max_callback_bindings=64\n",
    "state_transfer_migration_fuel=10000000\n",
    "reload_command_queue_capacity=1\n",
    "event_queue_capacity=8\n",
);

#[test]
fn cli_safe_profile_has_stable_versioned_text_and_round_trips() {
    let config = DevelopmentHostConfig::cli_safe_profile();

    assert_eq!(config.to_text().unwrap(), CLI_SAFE_TEXT);
    assert!(!CLI_SAFE_TEXT.contains("session_id"));
    assert!(!CLI_SAFE_TEXT.contains("session_token"));
    assert_eq!(
        CLI_SAFE_TEXT.parse::<DevelopmentHostConfig>().unwrap(),
        config
    );
    assert_eq!(
        DevelopmentHostConfig::from_text(CLI_SAFE_TEXT).unwrap(),
        config
    );
    assert_eq!(config.version(), DEVELOPMENT_HOST_CONFIG_VERSION);
    assert_eq!(config.runtime_fuel_per_call_limit(), 100_000_000);
    assert_eq!(config.module_bytes_limit(), 16 * 1_024 * 1_024);
    assert_eq!(config.runtime_memory_pages_limit(), 64);
    assert_eq!(config.runtime_table_elements_limit(), 2_048);
    assert_eq!(config.runtime_call_depth_limit(), 256);
    assert_eq!(config.protocol_chunk_bytes_limit(), 64 * 1_024);
    assert_eq!(config.protocol_diagnostic_bytes_limit(), 64 * 1_024);
    assert_eq!(config.protocol_terminal_result_limit(), 4);
    assert_eq!(config.protocol_io_timeout_ms_limit(), 10_000);
    assert_eq!(config.listener_credential_ttl_ms_limit(), 28_800_000);
    assert_eq!(config.listener_authentication_failure_limit(), 4);
    assert_eq!(config.listener_authentication_backoff_ms_limit(), 1_000);
    assert_eq!(config.model_document_bytes_limit(), 32 * 1_024);
    assert_eq!(config.model_state_entry_limit(), 512);
    assert_eq!(config.model_widget_node_limit(), 512);
    assert_eq!(config.model_property_limit(), 512);
    assert_eq!(config.model_widget_depth_limit(), 32);
    assert_eq!(config.retired_generation_limit(), 4);
    assert!(config.callback_binding_limit() >= 33);
    assert_eq!(config.state_transfer_migration_fuel_limit(), 10_000_000);
    assert_eq!(config.reload_command_queue_capacity_limit(), 1);
    assert_eq!(config.event_queue_capacity_limit(), 8);
}

#[test]
fn cli_safe_profile_can_hold_an_application_widget_document() {
    let config = DevelopmentHostConfig::cli_safe_profile();

    assert!(config.runtime_fuel_per_call_limit() >= 100_000_000);
    assert!(config.model_document_bytes_limit() >= 32 * 1_024);
    assert!(config.model_state_entry_limit() >= 512);
    assert!(config.model_widget_node_limit() >= 512);
    assert!(config.model_property_limit() >= 512);
    assert!(config.model_widget_depth_limit() >= 32);
}

#[test]
fn widget_ir_diagnostics_are_explicit_strict_and_round_trip() {
    let disabled = DevelopmentHostConfig::cli_safe_profile();
    let enabled = disabled.widget_ir_diagnostics(true);
    let enabled_text =
        CLI_SAFE_TEXT.replacen("widget_ir_diagnostics=0", "widget_ir_diagnostics=1", 1);

    assert!(!disabled.widget_ir_diagnostics_enabled());
    assert!(enabled.widget_ir_diagnostics_enabled());
    assert_eq!(disabled.to_text().unwrap(), CLI_SAFE_TEXT);
    assert_eq!(enabled.to_text().unwrap(), enabled_text);
    assert_eq!(
        CLI_SAFE_TEXT.parse::<DevelopmentHostConfig>().unwrap(),
        disabled
    );
    assert_eq!(
        enabled_text.parse::<DevelopmentHostConfig>().unwrap(),
        enabled
    );
    assert_eq!(disabled.validate().unwrap(), disabled);
    assert_eq!(enabled.validate().unwrap(), enabled);

    let missing = CLI_SAFE_TEXT.replacen("widget_ir_diagnostics=0\n", "", 1);
    let duplicate = format!("{CLI_SAFE_TEXT}widget_ir_diagnostics=0\n");
    assert!(matches!(
        missing.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::MissingField(
            "widget_ir_diagnostics"
        ))
    ));
    assert!(matches!(
        duplicate.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::DuplicateField(
            "widget_ir_diagnostics"
        ))
    ));
    for malformed in ["2", "true", "01"] {
        let text = CLI_SAFE_TEXT.replacen(
            "widget_ir_diagnostics=0",
            &format!("widget_ir_diagnostics={malformed}"),
            1,
        );
        assert!(matches!(
            text.parse::<DevelopmentHostConfig>(),
            Err(DevelopmentHostConfigError::MalformedValue(
                "widget_ir_diagnostics"
            ))
        ));
    }
}

#[test]
fn parser_rejects_missing_duplicate_unknown_and_malformed_fields() {
    let missing = CLI_SAFE_TEXT.replacen("event_queue_capacity=8\n", "", 1);
    let duplicate = format!("{CLI_SAFE_TEXT}event_queue_capacity=8\n");
    let unknown = format!("{CLI_SAFE_TEXT}session_token=secret\n");
    let malformed = CLI_SAFE_TEXT.replacen(
        "event_queue_capacity=8",
        "event_queue_capacity=not-a-number",
        1,
    );

    assert!(matches!(
        missing.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::MissingField(
            "event_queue_capacity"
        ))
    ));
    assert!(matches!(
        duplicate.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::DuplicateField(
            "event_queue_capacity"
        ))
    ));
    assert!(matches!(
        unknown.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::UnknownField)
    ));
    assert!(matches!(
        malformed.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::MalformedValue(
            "event_queue_capacity"
        ))
    ));
    let secret_error = unknown.parse::<DevelopmentHostConfig>().unwrap_err();
    assert!(!format!("{secret_error:?} {secret_error}").contains("secret"));
    for noncanonical in ["+8", "08", " 8", "8 "] {
        let text = CLI_SAFE_TEXT.replacen(
            "event_queue_capacity=8",
            &format!("event_queue_capacity={noncanonical}"),
            1,
        );
        assert!(matches!(
            text.parse::<DevelopmentHostConfig>(),
            Err(DevelopmentHostConfigError::MalformedValue(
                "event_queue_capacity"
            ))
        ));
    }
}

#[test]
fn parser_rejects_unknown_versions_zeroes_and_out_of_range_values() {
    let unknown_version = CLI_SAFE_TEXT.replacen("version=1", "version=2", 1);
    let zero = CLI_SAFE_TEXT.replacen("event_queue_capacity=8", "event_queue_capacity=0", 1);
    let out_of_range = CLI_SAFE_TEXT.replacen(
        "runtime_max_memory_pages=64",
        "runtime_max_memory_pages=65537",
        1,
    );
    let wider_than_destination = CLI_SAFE_TEXT.replacen(
        "runtime_max_memory_pages=64",
        "runtime_max_memory_pages=4294967296",
        1,
    );
    let inconsistent =
        CLI_SAFE_TEXT.replacen("max_module_bytes=16777216", "max_module_bytes=65535", 1);

    assert!(matches!(
        unknown_version.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::UnsupportedVersion(2))
    ));
    assert!(matches!(
        zero.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::OutOfRange {
            field: "event_queue_capacity",
            value: 0,
            minimum: 1,
            maximum: 65_536,
        })
    ));
    assert!(matches!(
        out_of_range.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::OutOfRange {
            field: "runtime_max_memory_pages",
            value: 65_537,
            minimum: 1,
            maximum: 65_536,
        })
    ));
    assert!(matches!(
        wider_than_destination.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::OutOfRange {
            field: "runtime_max_memory_pages",
            value: 4_294_967_296,
            minimum: 1,
            maximum: 65_536,
        })
    ));
    assert!(matches!(
        inconsistent.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::ExceedsField {
            field: "protocol_max_chunk_bytes",
            maximum_field: "max_module_bytes",
        })
    ));
}

#[test]
fn parser_bounds_input_before_processing_fields() {
    let oversized = "x".repeat(MAX_DEVELOPMENT_HOST_CONFIG_TEXT_BYTES + 1);

    assert!(matches!(
        oversized.parse::<DevelopmentHostConfig>(),
        Err(DevelopmentHostConfigError::TextTooLong { .. })
    ));
}

#[test]
fn serialization_revalidates_programmatic_profiles() {
    let invalid = DevelopmentHostConfig::cli_safe_profile().event_queue_capacity(0);

    assert!(matches!(
        invalid.to_text(),
        Err(DevelopmentHostConfigError::OutOfRange {
            field: "event_queue_capacity",
            value: 0,
            minimum: 1,
            maximum: 65_536,
        })
    ));
}

#[test]
fn every_programmatic_ceiling_rejects_zero_and_values_above_its_range() {
    type Setter = fn(DevelopmentHostConfig, u64) -> DevelopmentHostConfig;
    let cases: [(&str, u64, Setter); 22] = [
        ("runtime_fuel_per_call", MAX_FUEL, |config, value| {
            config.runtime_fuel_per_call(value)
        }),
        ("max_module_bytes", MAX_MODULE_BYTES, |config, value| {
            config.max_module_bytes(value as usize)
        }),
        (
            "runtime_max_memory_pages",
            MAX_MEMORY_PAGES,
            |config, value| config.runtime_max_memory_pages(value as u32),
        ),
        (
            "runtime_max_table_elements",
            MAX_TABLE_ELEMENTS,
            |config, value| config.runtime_max_table_elements(value as u32),
        ),
        ("runtime_max_call_depth", MAX_CALL_DEPTH, |config, value| {
            config.runtime_max_call_depth(value as u32)
        }),
        (
            "protocol_max_chunk_bytes",
            MAX_CHUNK_BYTES,
            |config, value| config.protocol_max_chunk_bytes(value as usize),
        ),
        (
            "protocol_max_diagnostic_bytes",
            MAX_DIAGNOSTIC_BYTES,
            |config, value| config.protocol_max_diagnostic_bytes(value as usize),
        ),
        (
            "protocol_max_terminal_results",
            MAX_TERMINAL_RESULTS,
            |config, value| config.protocol_max_terminal_results(value as usize),
        ),
        (
            "protocol_io_timeout_ms",
            MAX_IO_TIMEOUT_MS,
            |config, value| config.protocol_io_timeout_ms(value),
        ),
        (
            "listener_credential_ttl_ms",
            MAX_CREDENTIAL_TTL_MS,
            |config, value| config.listener_credential_ttl_ms(value),
        ),
        (
            "listener_max_authentication_failures",
            MAX_AUTHENTICATION_FAILURES,
            |config, value| config.listener_max_authentication_failures(value as u32),
        ),
        (
            "listener_authentication_backoff_ms",
            MAX_AUTHENTICATION_BACKOFF_MS,
            |config, value| config.listener_authentication_backoff_ms(value),
        ),
        (
            "model_max_document_bytes",
            MAX_MODEL_DOCUMENT_BYTES,
            |config, value| config.model_max_document_bytes(value as usize),
        ),
        (
            "model_max_state_entries",
            MAX_MODEL_ITEMS,
            |config, value| config.model_max_state_entries(value as u32),
        ),
        (
            "model_max_widget_nodes",
            MAX_MODEL_ITEMS,
            |config, value| config.model_max_widget_nodes(value as u32),
        ),
        (
            "model_max_properties",
            MAX_MODEL_PROPERTIES,
            |config, value| config.model_max_properties(value as u32),
        ),
        (
            "model_max_widget_depth",
            MAX_WIDGET_DEPTH,
            |config, value| config.model_max_widget_depth(value as u32),
        ),
        (
            "generation_max_retired",
            MAX_RETIRED_GENERATIONS,
            |config, value| config.generation_max_retired(value as usize),
        ),
        (
            "max_callback_bindings",
            MAX_CALLBACK_BINDINGS,
            |config, value| config.max_callback_bindings(value as u32),
        ),
        (
            "state_transfer_migration_fuel",
            MAX_FUEL,
            |config, value| config.state_transfer_migration_fuel(value),
        ),
        (
            "reload_command_queue_capacity",
            MAX_QUEUE_CAPACITY,
            |config, value| config.reload_command_queue_capacity(value as usize),
        ),
        (
            "event_queue_capacity",
            MAX_QUEUE_CAPACITY,
            |config, value| config.event_queue_capacity(value as usize),
        ),
    ];

    for (field, maximum, setter) in cases {
        assert_eq!(
            setter(DevelopmentHostConfig::cli_safe_profile(), 0).validate(),
            Err(DevelopmentHostConfigError::OutOfRange {
                field,
                value: 0,
                minimum: 1,
                maximum,
            })
        );
        assert_eq!(
            setter(DevelopmentHostConfig::cli_safe_profile(), maximum + 1).validate(),
            Err(DevelopmentHostConfigError::OutOfRange {
                field,
                value: maximum + 1,
                minimum: 1,
                maximum,
            })
        );
    }
}
