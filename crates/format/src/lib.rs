mod config;
#[cfg(not(target_family = "wasm"))]
mod discovery;
mod format;
#[cfg(not(target_family = "wasm"))]
mod ignore;
#[cfg(not(target_family = "wasm"))]
mod lsp;
mod oxfmtrc;
mod support;
mod utils;

#[cfg(feature = "napi")]
mod external_formatter;

pub use config::{
    ConfigResolver, ResolvedOptions, resolve_editorconfig_path, resolve_oxfmtrc_path,
};
#[cfg(not(target_family = "wasm"))]
pub use discovery::{
    FormatTargets, build_global_ignore_matchers, collect_matching_files, is_global_ignored,
    resolve_ignore_paths,
};
pub use format::{FormatResult, SourceFormatter};
#[cfg(not(target_family = "wasm"))]
pub use ignore::{build_ignore_matcher, is_gitignore_match};
#[cfg(not(target_family = "wasm"))]
pub use lsp::run_lsp;
pub use oxc_formatter_json::{JsonFormatOptions, JsonVariant};
pub use support::{FormatFileStrategy, JsonType, should_ignore_file};

#[cfg(feature = "napi")]
pub use external_formatter::{
    ExternalFormatter, JsFormatEmbeddedCb, JsFormatFileCb, JsInitExternalFormatterCb,
};
