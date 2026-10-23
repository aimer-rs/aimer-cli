pub mod assemble;
pub mod assets;
pub mod build;
pub mod clean;
pub mod completions;
pub mod create;
pub mod docs;
pub mod doctor;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "mcp")]
mod mcp_cargo;
pub mod mcp_docs;
#[cfg(feature = "mcp")]
pub mod mcp_install;
#[cfg(feature = "mcp")]
mod mcp_ra;
#[cfg(feature = "mcp")]
mod mcp_ra_engine;
pub mod migrate;
mod project_selection;
pub mod run;
pub mod sessions;
pub mod skill_install;
pub mod version;
