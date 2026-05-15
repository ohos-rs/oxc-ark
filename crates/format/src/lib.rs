mod config;
mod format;
#[cfg(not(target_family = "wasm"))]
mod lsp;
mod oxfmtrc;
mod support;
mod utils;

#[cfg(feature = "napi")]
mod external_formatter;

pub use config::{
    ConfigResolver, JsonFormatterOptions, ResolvedOptions, resolve_editorconfig_path,
    resolve_oxfmtrc_path,
};
pub use format::{FormatResult, SourceFormatter};
#[cfg(not(target_family = "wasm"))]
pub use lsp::run_lsp;
pub use support::{FormatFileStrategy, JsonType, should_ignore_file};

#[cfg(feature = "napi")]
pub use external_formatter::{
    ExternalFormatter, JsFormatEmbeddedCb, JsFormatFileCb, JsInitExternalFormatterCb,
};
