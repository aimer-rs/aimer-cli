//! Cargo project and manifest operations exposed by the MCP server.

mod common;
mod manifest;
mod metadata;
mod run;

pub use manifest::{
    ManifestEditArguments, ManifestReadArguments, prepare_manifest_edit, read_manifest,
};
#[cfg(test)]
pub(super) use metadata::read_project_metadata;
pub use metadata::{ProjectMetadataDetail, read_project_metadata_with_detail};
pub use run::{CargoRunArguments, CargoRunToolArguments, run_cargo, run_cargo_binary, run_clippy};

#[cfg(test)]
pub(super) use common::bounded_text;
#[cfg(test)]
pub(super) use manifest::{
    ManifestOperation, ManifestOperationAction, apply_manifest_operation, resolve_manifest,
};
#[cfg(test)]
pub(super) use metadata::project_metadata_value;
#[cfg(test)]
pub(super) use run::{normalize_cargo_output, validate_cargo_arguments};

#[cfg(test)]
mod tests;
