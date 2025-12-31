mod config;
mod format;
mod support;
mod utils;

#[cfg(feature = "napi")]
mod external_formatter;

pub use config::{
    ConfigResolver, JsonFormatterOptions, ResolvedOptions, resolve_editorconfig_path,
    resolve_oxfmtrc_path,
};
pub use format::{FormatResult, SourceFormatter};
pub use support::{FormatFileStrategy, JsonType, should_ignore_file};

#[cfg(feature = "napi")]
pub use external_formatter::{
    ExternalFormatter, JsFormatEmbeddedCb, JsFormatFileCb, JsInitExternalFormatterCb,
};
