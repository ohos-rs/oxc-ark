pub mod config;
pub mod embed;
pub mod format;
#[cfg(not(target_family = "wasm"))]
pub mod global_ignore;
pub mod language;
pub mod options;
pub mod oxfmtrc;
pub mod support;
#[cfg(feature = "napi")]
pub use crate::external_formatter::ExternalServices;
pub mod utils;

pub use config::{ConfigResolver, ConfigScopes, NestedConfigCtx, ResolveOutcome, config_discovery};
#[cfg(feature = "napi")]
pub use config::{JsConfigLoaderCb, JsLoadJsConfigCb, create_js_config_loader, resolve_for_api};
pub use language::Language;

pub use support::classify_file_with_language;
