use std::{fmt::Write, path::PathBuf, sync::Arc};

use oxc_language_server::{WorkerManager, run_server};
use oxc_linter::ExternalLinter;

mod code_actions;
mod commands;
mod error_with_position;
mod lsp_file_system;
mod server_linter;
mod utils;

pub mod options;

/// Run the language server
pub async fn run_lsp(external_linter: Option<ExternalLinter>, config_path: Option<PathBuf>) {
    let version = {
        let mut version = env!("CARGO_PKG_VERSION").to_string();
        if let Some(vp_version) = std::env::var_os("VP_VERSION") {
            let _ = write!(version, " (VP: {})", vp_version.to_string_lossy());
        }
        version
    };
    run_server(
        "oxk".to_string(),
        version,
        WorkerManager::new(Arc::new(server_linter::ServerLinterBuilder::new(
            external_linter,
            config_path,
        ))),
    )
    .await;
}
