//! Query registry or Rust toolchain documentation from the CLI.

use anyhow::Context;

use super::mcp_docs::{DocsRsClient, GetCrateDocsArguments, SearchCrateDocsArguments};

pub fn execute(
    crate_name: String,
    item_path: Option<String>,
    search: Option<String>,
    version: Option<String>,
    limit: Option<usize>,
    refresh: bool,
) -> anyhow::Result<()> {
    let project_root = std::env::current_dir().context("reading the current directory")?;
    let client = DocsRsClient::new(project_root)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating the docs.rs client runtime")?;

    let result = if let Some(query) = search {
        serde_json::to_value(runtime.block_on(client.search_documents(
            SearchCrateDocsArguments {
                crate_name,
                query,
                version,
                limit,
                refresh,
            },
        ))?)?
    } else {
        serde_json::to_value(runtime.block_on(client.get_document(GetCrateDocsArguments {
            crate_name,
            item_path,
            version,
            refresh,
        }))?)?
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
